"""Local Windows wish-history import probe; URLs and credentials are never saved."""
import argparse
import ctypes
import http.client
import json
import os
from pathlib import Path
import re
import socket
import struct
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from datetime import datetime, timezone


API = "https://public-operation-hk4e-sg.hoyoverse.com/gacha_info/api/getGachaLog"
REGIONS = ("os_euro", "os_asia", "os_usa", "os_cht")
BANNERS = ("100", "200", "301", "302", "500")
PARAMS = ("authkey", "authkey_ver", "sign_type", "auth_appid", "game_biz", "region", "lang", "init_type")
_last_peer = {}
_interface_index = None


class ImportFailure(Exception):
    """Only explicitly safe messages may be returned to the console."""


class NetworkFailure(ImportFailure):
    """The API is unreachable; refreshing cache credentials cannot fix routing."""


class ApiFailure(ImportFailure):
    def __init__(self, code):
        self.code = code
        super().__init__(f"History API rejected the request (code {code}); reopen the in-game history")


@dataclass(repr=False)
class Access:
    params: dict
    cache_time: float
    offset: int
    timestamp: int


def shared_read(path, limit=32 * 1024 * 1024):
    if os.name != "nt":
        with path.open("rb") as stream:
            data = stream.read(limit + 1)
    else:
        from ctypes import wintypes as w
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.CreateFileW.argtypes = [w.LPCWSTR, w.DWORD, w.DWORD, w.LPVOID, w.DWORD, w.DWORD, w.HANDLE]
        kernel.CreateFileW.restype = w.HANDLE
        kernel.ReadFile.argtypes = [w.HANDLE, w.LPVOID, w.DWORD, ctypes.POINTER(w.DWORD), w.LPVOID]
        kernel.ReadFile.restype = w.BOOL
        kernel.CloseHandle.argtypes = [w.HANDLE]
        handle = kernel.CreateFileW(str(path), 0x80000000, 7, None, 3, 0, None)
        if handle == ctypes.c_void_p(-1).value:
            raise ImportFailure("Cannot read game cache with shared access")
        try:
            chunks = []
            size = 0
            while size <= limit:
                buffer = ctypes.create_string_buffer(min(1024 * 1024, limit + 1 - size))
                count = w.DWORD()
                if not kernel.ReadFile(handle, buffer, len(buffer), ctypes.byref(count), None):
                    raise ImportFailure("Cannot read game cache")
                if not count.value:
                    break
                chunks.append(buffer.raw[:count.value])
                size += count.value
            data = b"".join(chunks)
        finally:
            kernel.CloseHandle(handle)
    if len(data) > limit:
        raise ImportFailure("Cache exceeds the bounded scan size")
    return data


def find_game_data(app_data=None):
    # Portable setup already knows the installation; the game log is only a fallback.
    if app_data is not None:
        config = Path(app_data) / "body-codec.json"
        if config.is_file():
            try:
                client = json.loads(shared_read(config, 4096)).get("client_directory")
                if isinstance(client, str) and Path(client).is_absolute():
                    root = Path(client) / "GenshinImpact_Data"
                    if root.is_dir():
                        return root
            except (ImportFailure, OSError, ValueError, AttributeError):
                pass
    log = Path(os.environ.get("USERPROFILE", str(Path.home()))) / "AppData/LocalLow/miHoYo/Genshin Impact/output_log.txt"
    if not log.is_file():
        raise ImportFailure("Game log not found; specify --game-data")
    text = shared_read(log).decode("utf-8", errors="replace")
    for match in re.finditer(r"[A-Za-z]:[/\\][^\r\n\x00]*?GenshinImpact_Data", text):
        path = Path(match.group())
        if path.is_dir():
            return path
    raise ImportFailure("Game data path not found; specify --game-data")


def cache_candidates(data, cache_time=0):
    result = []
    for match in re.finditer(rb"https://[\x21-\x7e]{1,20000}", data):
        try:
            raw = match.group().decode("ascii").split('"')[0].split("<")[0].replace("&amp;", "&")
            parts = urllib.parse.urlsplit(raw)
            # Cache strings are untrusted. Never use their host as a request destination.
            if not parts.hostname or not any(parts.hostname == host or parts.hostname.endswith("." + host) for host in ("hoyoverse.com", "mihoyo.com")):
                continue
            values = urllib.parse.parse_qs(parts.query)
            if any(len(v) != 1 for k, v in values.items() if k in PARAMS):
                continue
            query = {k: v[0] for k, v in values.items()}
            if query.get("auth_appid") != "webview_gacha" or not query.get("authkey") or query.get("game_biz") != "hk4e_global" or query.get("region") not in REGIONS:
                continue
            timestamp = query.get("timestamp", "0")
            result.append(Access({k: query[k] for k in PARAMS if k in query}, cache_time, match.start(), int(timestamp) if timestamp.isdecimal() else 0))
        except (ValueError, UnicodeError):
            continue
    return result


def discover(root, region=None):
    candidates = []
    files = list(root.glob("webCaches/**/Cache/Cache_Data/data_2"))
    for path in files:
        candidates.extend(cache_candidates(shared_read(path), path.stat().st_mtime))
    candidates.sort(key=lambda c: (c.timestamp, c.cache_time, c.offset), reverse=True)
    unique = []
    seen = set()
    for candidate in candidates:
        identity = (candidate.params["authkey"], candidate.params["region"])
        if identity not in seen and (region is None or identity[1] == region):
            seen.add(identity)
            unique.append(candidate)
    return unique, len(files)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise ImportFailure("History API redirect rejected")


def bounded_connect(address, timeout=10, source_address=None):
    host, port = address
    deadline = time.monotonic() + timeout
    addresses = socket.getaddrinfo(host, port, type=socket.SOCK_STREAM)
    addresses.sort(key=lambda entry: entry[4][0] != _last_peer.get(host))
    for family, kind, protocol, _, peer in addresses:
        if _interface_index is not None and family != socket.AF_INET:
            continue
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            break
        connection = socket.socket(family, kind, protocol)
        try:
            connection.settimeout(min(2, remaining))
            if _interface_index is not None:
                # Windows IP_UNICAST_IF affects this API socket only; no route/VPN changes.
                connection.setsockopt(socket.IPPROTO_IP, 31, struct.pack("!I", _interface_index))
            if source_address:
                connection.bind(source_address)
            connection.connect(peer)
            connection.settimeout(max(0.1, deadline - time.monotonic()))
            _last_peer[host] = peer[0]
            return connection
        except OSError:
            connection.close()
    raise TimeoutError("History endpoint connection failed")


class HistoryConnection(http.client.HTTPSConnection):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self._create_connection = bounded_connect


class HistoryHTTPS(urllib.request.HTTPSHandler):
    def https_open(self, request):
        return self.do_open(HistoryConnection, request, context=self._context)


def request_page(access, banner, page, cursor, timeout=10):
    query = dict(access.params)
    query.update(gacha_type=banner, page=str(page), size="20", end_id=cursor)
    url = API + "?" + urllib.parse.urlencode(query)
    request = urllib.request.Request(url, headers={"User-Agent": "VisionOverlay-WishImport/0.1", "Accept": "application/json"})
    try:
        with urllib.request.build_opener(NoRedirect(), HistoryHTTPS()).open(request, timeout=timeout) as response:
            body = response.read(4 * 1024 * 1024 + 1)
        if len(body) > 4 * 1024 * 1024:
            raise ImportFailure("History response exceeds size limit")
        data = json.loads(body)
    except ImportFailure:
        raise
    except urllib.error.HTTPError as error:
        raise ImportFailure(f"History API HTTP error {error.code}") from None
    except (urllib.error.URLError, TimeoutError, OSError):
        raise NetworkFailure("Cannot connect to HoYoVerse history API; check whether history loads in the game") from None
    except (ValueError, UnicodeError):
        raise ImportFailure("Invalid history API JSON") from None
    if not isinstance(data, dict) or data.get("retcode") != 0:
        code = data.get("retcode") if isinstance(data, dict) else None
        safe_code = code if type(code) is int else "unknown"
        raise ApiFailure(safe_code)
    return data.get("data")


def select_access(candidates, uid, region, request=request_page, pause=time.sleep, progress=print):
    # Cache offsets and URL timestamps do not prove token freshness or account identity.
    # Validate a bounded number of candidates, without logging URLs or other UIDs.
    for index, candidate in enumerate(candidates[:12], 1):
        if candidate.params.get("region") != region:
            continue
        progress(f"Checking cached access candidate {index}")
        for banner in ("301", "200", "100", "302", "500"):
            try:
                payload = request(candidate, banner, 1, "0")
            except ApiFailure as error:
                if error.code in (-100, -101):
                    pause(0.6)
                    break
                raise
            if not isinstance(payload, dict) or not isinstance(payload.get("list"), list) or len(payload["list"]) > 20:
                raise ImportFailure("Invalid history access-check response")
            if payload.get("region") not in (None, region):
                pause(0.6)
                break
            rows = payload["list"]
            if rows:
                if all(isinstance(row, dict) and row.get("uid") == uid for row in rows):
                    progress("Cached access verified for the requested UID and region")
                    pause(0.6)
                    return candidate, (banner, payload)
                pause(0.6)
                break
            pause(0.6)
    raise ImportFailure("No valid cached access matched this UID; reopen its in-game history")


def record(raw, uid):
    if not isinstance(raw, dict) or raw.get("uid") != uid:
        raise ImportFailure("History belongs to another UID or has no verifiable UID; nothing was saved")
    row = {k: raw.get(k) for k in ("id", "uid", "gacha_type", "item_id", "count", "time", "name", "item_type", "rank_type", "lang")}
    if not isinstance(row["id"], str) or not re.fullmatch(r"[1-9][0-9]{0,31}", row["id"]):
        raise ImportFailure("History record has no valid stable ID")
    if row["gacha_type"] not in (*BANNERS, "400") or row["rank_type"] not in ("3", "4", "5"):
        raise ImportFailure("Unsupported history record type")
    if row["count"] != "1" or not isinstance(row["name"], str) or len(row["name"]) > 512:
        raise ImportFailure("Invalid history item")
    if not isinstance(row["time"], str) or not re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2}", row["time"]):
        raise ImportFailure("Invalid history timestamp")
    try:
        datetime.strptime(row["time"], "%Y-%m-%d %H:%M:%S")
    except (ValueError, TypeError):
        raise ImportFailure("Invalid history timestamp") from None
    return row


def fetch(access, uid, region, request=request_page, pause=time.sleep, progress=print, banners=BANNERS):
    if region not in REGIONS or access.params.get("region") != region:
        raise ImportFailure("Cached URL does not match the requested region")
    records = {}
    metadata = []
    for banner in banners:
        cursor = "0"
        seen_cursors = set()
        region_proof = None
        zone = None
        for page in range(1, 10001):
            payload = request(access, banner, page, cursor)
            if not isinstance(payload, dict) or not isinstance(payload.get("list"), list) or len(payload["list"]) > 20:
                raise ImportFailure("Invalid history page")
            response_region = payload.get("region")
            if response_region is not None and response_region != region:
                raise ImportFailure("History API returned another region; nothing was saved")
            if response_region is not None:
                region_proof = response_region
            if payload.get("region_time_zone") is not None:
                try:
                    new_zone = int(payload["region_time_zone"])
                except (ValueError, TypeError):
                    raise ImportFailure("Invalid history server timezone") from None
                if not -12 <= new_zone <= 14 or (zone is not None and zone != new_zone):
                    raise ImportFailure("Inconsistent history server timezone")
                zone = new_zone
            rows = [record(row, uid) for row in payload["list"]]
            if any(row["gacha_type"] not in (("301", "400") if banner == "301" else (banner,)) for row in rows):
                raise ImportFailure("History API returned another banner group")
            if not rows:
                metadata.append({"gacha_type": banner, "pages": page, "region_from_api": region_proof, "region_time_zone": zone})
                break
            ids = [int(row["id"]) for row in rows]
            if len(set(ids)) != len(ids) or ids != sorted(ids, reverse=True) or (cursor != "0" and max(ids) >= int(cursor)):
                raise ImportFailure("History cursor did not advance; nothing was saved")
            for row in rows:
                if row["id"] in records and records[row["id"]] != row:
                    raise ImportFailure("Conflicting history record ID")
                records[row["id"]] = row
            cursor = rows[-1]["id"]
            if cursor in seen_cursors:
                raise ImportFailure("Repeated history cursor")
            seen_cursors.add(cursor)
            if page == 1 or page % 20 == 0:
                progress(f"Banner {banner}, page {page}: {len(records)} unique records")
            pause(0.6)
        else:
            raise ImportFailure("History page limit reached; nothing was saved")
    if not records:
        raise ImportFailure("No records returned: cannot verify UID or declare a complete account history")
    zones = {stream["region_time_zone"] for stream in metadata if stream["region_time_zone"] is not None}
    if len(zones) > 1:
        raise ImportFailure("History streams have inconsistent server timezones")
    values = sorted(records.values(), key=lambda r: int(r["id"]))
    return {"schema_version": 1, "source": "hoyoverse_wish_api", "game_id": "genshin", "account_namespace": "hk4e_global", "uid": uid, "server_region": region,
            "region_evidence": "api_response" if any(s["region_from_api"] == region for s in metadata) else "cache_url_and_explicit_selection",
            "region_time_zone": next(iter(zones)) if zones else None,
            "fetched_at": datetime.now(timezone.utc).isoformat(), "streams": metadata,
            "coverage": {"all_available_pages_fetched": True, "complete_lifetime_history": False,
                         "earliest": min(r["time"] for r in values), "latest": max(r["time"] for r in values)},
            "records": values}


def main():
    global _interface_index
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--game-data", type=Path)
    parser.add_argument("--expected-uid")
    parser.add_argument("--expected-region", choices=REGIONS)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--inspect", action="store_true", help="Only inspect cache metadata; no network requests")
    parser.add_argument("--interface-index", type=int, help="Windows: use this outgoing IPv4 interface for API sockets only")
    args = parser.parse_args()
    if args.interface_index is not None:
        if os.name != "nt" or not 1 <= args.interface_index <= 0xFFFFFF:
            parser.error("--interface-index requires a valid Windows interface index")
        _interface_index = args.interface_index
    if not args.inspect and (not args.expected_uid or not re.fullmatch(r"[1-9][0-9]{5,19}", args.expected_uid) or not args.expected_region):
        parser.error("--expected-uid and --expected-region are required for import")
    candidates, files = discover(args.game_data or find_game_data(), args.expected_region)
    if args.inspect:
        print(json.dumps({"cache_files": files, "unique_access_candidates": len(candidates), "regions": sorted({c.params["region"] for c in candidates})}))
        return
    if not candidates:
        raise ImportFailure("No matching cached history URL; open the in-game history first")
    output = args.output or Path("local-data/wishes") / args.expected_region / args.expected_uid / ("import-" + datetime.now(timezone.utc).strftime("%Y%m%d-%H%M%S-%f") + ".json")
    if output.exists():
        raise ImportFailure("Output already exists; existing history was not changed")
    candidate, (first_banner, first_page) = select_access(candidates, args.expected_uid, args.expected_region)
    def request(context, banner, page, cursor):
        if (banner, page, cursor) == (first_banner, 1, "0"):
            return first_page
        return request_page(context, banner, page, cursor)
    data = fetch(candidate, args.expected_uid, args.expected_region, request=request)
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("x", encoding="utf-8") as stream:
        json.dump(data, stream, ensure_ascii=False, indent=2)
        stream.write("\n")
    print(f"Saved {len(data['records'])} records: {output}")


if __name__ == "__main__":
    try:
        main()
    except ImportFailure as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
    except Exception as error:
        # Exception strings and tracebacks can contain authenticated URLs.
        print(f"Import failed ({type(error).__name__}); no complete import was saved", file=sys.stderr)
        sys.exit(1)
