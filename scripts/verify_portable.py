"""Exercise a packaged application in isolation, without capture or scheduled tasks."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import tempfile
import time
import urllib.request
import zipfile


def request(port, path):
    with urllib.request.urlopen(f"http://127.0.0.1:{port}{path}", timeout=2) as response:
        return json.load(response)


def pe_imports(path):
    data = path.read_bytes()
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    assert data[pe:pe + 4] == b"PE\0\0"
    machine, sections = struct.unpack_from("<HH", data, pe + 4)
    assert machine == 0x8664
    optional = pe + 24
    assert struct.unpack_from("<H", data, optional)[0] == 0x20B
    size = struct.unpack_from("<H", data, pe + 20)[0]
    table = optional + size

    def offset(rva):
        for i in range(sections):
            _, virtual_size, virtual, raw_size, raw = struct.unpack_from("<8sIIII", data, table + i * 40)
            if virtual <= rva < virtual + max(virtual_size, raw_size):
                return raw + rva - virtual
        raise AssertionError("Unmapped PE RVA")

    rva = struct.unpack_from("<I", data, optional + 112 + 8)[0]
    imports = []
    pos = offset(rva)
    while any(data[pos:pos + 20]):
        name = offset(struct.unpack_from("<I", data, pos + 12)[0])
        imports.append(data[name:data.index(0, name)].decode("ascii"))
        pos += 20
    return imports


def verify(archive, output, headless=False):
    output.mkdir(parents=True, exist_ok=False)
    output = output.resolve()
    with zipfile.ZipFile(archive) as z:
        assert z.testzip() is None
        for entry in z.infolist():
            target = (output / entry.filename).resolve()
            assert target.is_relative_to(output)
            assert "/data/" not in entry.filename.replace("\\", "/")
            assert Path(entry.filename).name not in ("collector-token.txt", "body-codec.json")
            if Path(entry.filename).name == "gi.json":
                assert entry.filename.replace("\\", "/").endswith("/components/reference/gi.json")
        z.extractall(output)
    roots = [p for p in output.iterdir() if p.is_dir()]
    assert len(roots) == 1
    root = roots[0]
    manifest = json.loads((root / "package-manifest.json").read_text(encoding="utf-8-sig"))
    for file in manifest["files"]:
        path = (root / file["path"]).resolve()
        assert path.is_relative_to(root.resolve())
        data = path.read_bytes()
        assert len(data) == file["bytes"]
        assert hashlib.sha256(data).hexdigest() == file["sha256"]
    dispatch = root / "components/reference/gi.json"
    assert hashlib.sha256(dispatch.read_bytes()).hexdigest() == "e0e1fcbfb6aa5d727367a60574b7688a4da14abe12c5a3bdad3a7fc87c694d18"
    assert (root / "licenses/Irminsul-LICENSE.txt").is_file()
    env = {**os.environ, "PATH": str(Path(os.environ["WINDIR"]) / "System32")}
    env.pop("PYTHONHOME", None)
    env.pop("PYTHONPATH", None)
    cwd = output / "arbitrary working directory"
    cwd.mkdir()
    python = root / "components/python/python.exe"
    subprocess.run([str(python), "-c", "import sync_wish_history,ctypes,ssl,json; print('Embedded Python imports OK')"], env=env, cwd=cwd, check=True, timeout=10)
    subprocess.run([str(root / "components/capture-probe.exe"), "--demo", "--output", str(output / "demo.jsonl")], env=env, cwd=cwd, check=True, timeout=10, capture_output=True)
    imports = {name: pe_imports(root / path) for name, path in [
        ("launcher", "VisionOverlay.exe"), ("server", "components/vision-server.exe"), ("capture", "components/capture-probe.exe")]}
    for names in imports.values():
        assert not any(n.lower().startswith(("vcruntime", "msvcp", "python", "sqlite3")) for n in names)
    # Reserve a separate port; never disturb the developer's server or game capture.
    with socket.socket() as reserve:
        reserve.bind(("127.0.0.1", 0))
        port = reserve.getsockname()[1]
    (root / "data").mkdir()
    (root / "data/launcher.json").write_text(json.dumps({"port": port}))

    def start(current):
        exe = current / "VisionOverlay.exe"
        subprocess.run([str(exe), "--check"], env=env, cwd=cwd, check=True, timeout=10)
        child = subprocess.Popen([str(exe), "--no-browser"] + (["--headless"] if headless else []), env=env, cwd=cwd)
        try:
            end = time.monotonic() + 50
            while True:
                assert child.poll() is None, "Launcher exited during startup"
                try:
                    state = request(port, "/api/v1/state")
                    break
                except (OSError, TimeoutError):
                    assert time.monotonic() < end, "Startup timed out"
                    time.sleep(.2)
            assert state["profiles"] == []
            catalog = request(port, "/catalog.json")
            for section, table in [("characterDetails", "skills"), ("characterDetails", "passives"), ("characterDetails", "constellations"), ("equipmentDetails", "weapons"), ("equipmentDetails", "sets"), ("progression", "avatarPaths"), ("progression", "talentLevels"), ("progression", "weaponPaths"), ("leveling", "avatarExp"), ("leveling", "weaponExp"), ("crafting", "recipes")]:
                assert catalog.get(section, {}).get(table), f"Fresh portable install has no {section}.{table}"

            assert (current / "data/vision-overlay.sqlite3").is_file()
            assert hashlib.sha256((current / "data/gi.json").read_bytes()).hexdigest() == "e0e1fcbfb6aa5d727367a60574b7688a4da14abe12c5a3bdad3a7fc87c694d18"
            subprocess.run([str(exe), "--no-browser"], env=env, cwd=cwd, check=True, timeout=10)
            assert child.poll() is None
            return child
        except BaseException:
            (current / "data/launcher-stop").write_text("stop")
            child.wait(timeout=25)
            raise

    def stop(current, child):
        subprocess.run([str(current / "VisionOverlay.exe"), "--stop"], env=env, cwd=cwd, check=True, timeout=10)
        assert child.wait(timeout=25) == 0
        with socket.socket() as probe:
            assert probe.connect_ex(("127.0.0.1", port)) != 0, "Server outlived launcher"

    child = start(root)
    stop(root, child)
    sentinel = root / "data/portable-test.txt"
    sentinel.write_text("preserve me")
    moved = output / "Переносимая версия с пробелами"
    assert root.resolve().is_relative_to(output) and moved.resolve().is_relative_to(output)
    root.rename(moved)
    child = start(moved)
    assert (moved / "data/portable-test.txt").read_text() == "preserve me"
    stop(moved, child)
    report = {"archive": archive.name, "files": len(manifest["files"]), "hashes": "passed", "isolated_runtime": "passed", "first_launch_empty_database": "passed", "complete_bootstrap_catalog": "passed", "bundled_dispatch_first_launch": "passed", "duplicate_instance": "passed", "graceful_shutdown": "passed", "relocation_unicode_path": "passed", "external_imports": imports, "test_directory": str(output)}
    (output / "verification.json").write_text(json.dumps(report, indent=2, ensure_ascii=False), encoding="utf-8")
    print(json.dumps(report, indent=2, ensure_ascii=False))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--headless", action="store_true", help="CI without Explorer/notification area")
    args = parser.parse_args()
    verify(args.archive.resolve(), args.output, args.headless)
