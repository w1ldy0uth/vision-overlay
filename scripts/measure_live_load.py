"""Read-only Windows soak sampler; no packet payloads, keys or account names saved.

Run from the repository root. Stop by creating OUTPUT/stop, or Ctrl+C.
CPU percent is normalized to the logical processor count; one_core_percent is
also retained. Samples append durably; summary.json can be read while running.
"""
import argparse
import datetime as dt
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import time
import urllib.request


def read_metadata(path):
    """Windows atomic-file readers must allow delete/rename by the writer."""
    if os.name != "nt":
        return path.read_text(encoding="utf-8-sig")
    import ctypes
    from ctypes import wintypes
    import msvcrt
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                                  ctypes.c_void_p, wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
    kernel.CreateFileW.restype = wintypes.HANDLE
    handle = kernel.CreateFileW(str(path), 0x80000000, 7, None, 3, 0x80, None)
    if handle == ctypes.c_void_p(-1).value:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        fd = msvcrt.open_osfhandle(handle, os.O_RDONLY)
    except Exception:
        kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        kernel.CloseHandle(handle)
        raise
    with os.fdopen(fd, "r", encoding="utf-8-sig") as stream:
        return stream.read()


def percentile(values, fraction):
    return sorted(values)[min(len(values) - 1, int((len(values) - 1) * fraction))] if values else None


def fetch(base, path):
    started = time.perf_counter()
    try:
        with urllib.request.urlopen(base + path, timeout=5) as response:
            data = json.load(response)
        return data, {"path": path, "ms": round((time.perf_counter() - started) * 1000, 2), "ok": True}
    except Exception as error:
        return None, {"path": path, "ms": round((time.perf_counter() - started) * 1000, 2), "ok": False, "error": type(error).__name__}


def size(path):
    try:
        return path.stat().st_size
    except FileNotFoundError:
        return 0


def sample_processes(previous, elapsed):
    command = "Get-Process vision-server,capture-probe -ErrorAction SilentlyContinue | Select-Object ProcessName,Id,CPU,WorkingSet64,PrivateMemorySize64,HandleCount,StartTime | ConvertTo-Json -Compress"
    result = subprocess.run(["powershell.exe", "-NoProfile", "-Command", command], capture_output=True, text=True, timeout=10)
    rows = json.loads(result.stdout) if result.stdout.strip() else []
    if isinstance(rows, dict):
        rows = [rows]
    current = {}
    for row in rows:
        key = str(row["Id"]) + str(row["StartTime"])
        cpu = row["CPU"]
        if cpu is not None:
            if key in previous:
                row["one_core_percent"] = round(max(0, cpu - previous[key]) / elapsed * 100, 3)
                row["machine_cpu_percent"] = round(row["one_core_percent"] / (os.cpu_count() or 1), 3)
            current[key] = cpu
    return rows, current


def capture_metrics(root, folder, tail_offset, telemetry):
    journal = folder / "capture.jsonl"
    # Incremental parse: ignore large snapshot payloads and retain only diagnostics.
    with journal.open("rb") as stream:
        stream.seek(tail_offset)
        while True:
            start = stream.tell()
            line = stream.readline()
            if not line or not line.endswith(b"\n"):
                tail_offset = start
                break
            tail_offset = stream.tell()
            value = json.loads(line)
            event = value.get("event")
            if event in ("source_summary", "status", "decoder_warning_counts"):
                telemetry[event] = {"data": value.get("data"), "timestamp": value.get("timestamp")}
    result = {"folder": folder.name, "journal_bytes": size(journal), "outbox_bytes": size(folder / "delivery/outbox.jsonl"), "diagnostics": dict(telemetry)}
    try:
        ack = json.loads(read_metadata(folder / "delivery/ack.json"))
        result["delivery_backlog_bytes"] = max(0, result["journal_bytes"] - ack["source_offset"])
    except (OSError, ValueError, KeyError):
        result["delivery_backlog_bytes"] = None
    try:
        manifest = json.loads(read_metadata(folder / "raw-ring/manifest.json"))
        result["ring"] = {key: manifest.get(key) for key in ("budget_bytes", "segment_limit_bytes", "evicted_segments", "evicted_packets", "closed")}
        result["ring"]["disk_bytes"] = sum(size(p) for p in (folder / "raw-ring").glob("segment-*.voraw"))
    except (OSError, ValueError):
        pass
    return result, tail_offset


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--minutes", type=float, default=480)
    parser.add_argument("--interval", type=float, default=15)
    parser.add_argument("--base", default="http://127.0.0.1:18788")
    args = parser.parse_args()
    if args.interval < 5 or args.minutes <= 0:
        parser.error("interval must be >=5 seconds and minutes >0")
    root = Path(__file__).resolve().parents[1]
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    if (out / "samples.jsonl").exists():
        parser.error("use a fresh output directory")
    previous, offsets, telemetry, all_api = {}, {}, {}, []
    start = last = time.monotonic()
    count = 0
    latest = None
    seen_capture = False
    absent_since = None
    try:
        with (out / "samples.jsonl").open("a", encoding="utf-8", buffering=1) as log:
            while time.monotonic() - start < args.minutes * 60 and not (out / "stop").exists():
                tick = time.monotonic()
                sample = {"utc": dt.datetime.now(dt.timezone.utc).isoformat(), "elapsed_seconds": round(tick - start, 2)}
                try:
                    sample["processes"], previous = sample_processes(previous, max(tick - last, 0.001))
                    capturing = any(p["ProcessName"] == "capture-probe" for p in sample["processes"])
                    if capturing:
                        seen_capture = True
                        absent_since = None
                    elif seen_capture and absent_since is None:
                        absent_since = tick
                    state, timing = fetch(args.base, "/api/v1/state")
                    timings = [timing]
                    if state:
                        sample["revision"] = state.get("revision")
                        profile = state.get("active_game_profile_id")
                        if profile:
                            for suffix in ("inventory?limit=50", "characters?limit=50", "events?limit=50"):
                                _, timing = fetch(args.base, f"/api/v1/profiles/{profile}/{suffix}")
                                # Avoid persisting account-specific route identities.
                                timing["path"] = suffix
                                timings.append(timing)
                    sample["api"] = timings
                    all_api.extend(timings)
                    db = root / "local-data/vision-overlay.sqlite3"
                    sample["database_bytes"] = size(db)
                    sample["wal_bytes"] = size(Path(str(db) + "-wal"))
                    folders = sorted((p for p in (root / "local-data").glob("realtime-*") if p.is_dir() and (p / "capture.jsonl").is_file()), key=lambda p: p.name)
                    if folders:
                        folder = folders[-1]
                        key = folder.name
                        sample["capture"], offsets[key] = capture_metrics(root, folder, offsets.get(key, 0), telemetry.setdefault(key, {}))
                    if count % 20 == 0:
                        with sqlite3.connect(db.as_uri() + "?mode=ro", uri=True, timeout=1) as conn:
                            sample["database_pages"] = dict(zip(("pages", "free_pages"), (conn.execute("pragma page_count").fetchone()[0], conn.execute("pragma freelist_count").fetchone()[0])))
                    sample["sampler_ms"] = round((time.monotonic() - tick) * 1000, 1)
                except Exception as error:
                    sample["sampler_error"] = type(error).__name__ + ": " + str(error)
                log.write(json.dumps(sample, ensure_ascii=False) + "\n")
                count += 1
                latest = sample
                good = [r["ms"] for r in all_api if r["ok"]]
                summary = {"samples": count, "elapsed_seconds": sample["elapsed_seconds"], "api_requests": len(all_api), "api_errors": sum(not r["ok"] for r in all_api), "api_p50_ms": percentile(good, .5), "api_p95_ms": percentile(good, .95), "latest": latest, "running": True}
                temp = out / "summary.tmp"
                temp.write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8")
                temp.replace(out / "summary.json")
                if absent_since is not None and tick - absent_since >= 120:
                    break
                last = tick
                time.sleep(max(0, args.interval - (time.monotonic() - tick)))
    except KeyboardInterrupt:
        pass
    finally:
        if (out / "summary.json").exists():
            summary = json.loads((out / "summary.json").read_text(encoding="utf-8"))
            summary["running"] = False
            (out / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8")


if __name__ == "__main__":
    main()
