import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch, MagicMock
from urllib.parse import urlencode
from urllib.error import URLError

import import_wish_history as wish


UID = "700000001"
REGION = "os_euro"


def access():
    return wish.Access({"authkey": "synthetic-secret+value", "region": REGION}, 0, 0, 0)


def row(id="1700000000000000001", banner="301", **changes):
    return dict(id=id, uid=UID, gacha_type=banner, item_id="", count="1", time="2026-11-12 17:00:00", name="Предмет", item_type="Оружие", rank_type="3", lang="ru", **changes)


class ImportTests(unittest.TestCase):
    def test_explicit_interface_is_applied_only_to_the_request_socket(self):
        connection = MagicMock()
        peers = [(2, 1, 6, "", ("192.0.2.1", 443))]
        with patch.object(wish, "_interface_index", 19), patch.object(wish.socket, "getaddrinfo", return_value=peers), patch.object(wish.socket, "socket", return_value=connection):
            wish.bounded_connect(("example.invalid", 443), 10)
            connection.setsockopt.assert_called_once_with(wish.socket.IPPROTO_IP, 31, wish.struct.pack("!I", 19))
        wish._last_peer.clear()

    def test_connection_skips_unreachable_resolved_address_and_reuses_successful_peer(self):
        first = MagicMock()
        first.connect.side_effect = TimeoutError()
        second = MagicMock()
        peers = [(2, 1, 6, "", ("192.0.2.1", 443)), (2, 1, 6, "", ("192.0.2.2", 443))]
        wish._last_peer.clear()
        with patch.object(wish.socket, "getaddrinfo", return_value=peers), patch.object(wish.socket, "socket", side_effect=[first, second]):
            self.assertIs(wish.bounded_connect(("example.invalid", 443), 10), second)
            first.close.assert_called_once()
            self.assertEqual(wish._last_peer["example.invalid"], "192.0.2.2")
            self.assertLessEqual(first.settimeout.call_args_list[0].args[0], 2)
        third = MagicMock()
        with patch.object(wish.socket, "getaddrinfo", return_value=peers), patch.object(wish.socket, "socket", return_value=third):
            wish.bounded_connect(("example.invalid", 443), 10)
            third.connect.assert_called_once_with(("192.0.2.2", 443))
        wish._last_peer.clear()

    def test_access_selection_skips_expired_and_other_accounts_before_verified_candidate(self):
        candidates = [access() for _ in range(4)]
        def request(context, *_):
            index = next(i for i, candidate in enumerate(candidates) if candidate is context)
            if index < 2:
                raise wish.ApiFailure(-101)
            raw = row()
            if index == 2:
                raw["uid"] = "700000002"
            return {"region": REGION, "list": [raw]}
        chosen, first = wish.select_access(candidates, UID, REGION, request, lambda _: None, lambda _: None)
        self.assertIs(chosen, candidates[3])
        self.assertEqual(first[0], "301")

    def test_access_selection_does_not_retry_transport_failure_or_rate_limit(self):
        for failure in (wish.ImportFailure("Cannot connect"), wish.ApiFailure(-110)):
            with patch.object(wish, "request_page") as unused:
                def request(*_):
                    raise failure
                with self.assertRaises(wish.ImportFailure):
                    wish.select_access([access()], UID, REGION, request, lambda _: None, lambda _: None)
                unused.assert_not_called()

    def test_cache_credentials_are_decoded_but_not_exposed_and_untrusted_hosts_are_rejected(self):
        query = urlencode({"auth_appid": "webview_gacha", "authkey": "synthetic-secret+value", "game_biz": "hk4e_global", "region": REGION, "timestamp": "123"})
        valid = ("https://webstatic-sea.hoyoverse.com/history?" + query).encode()
        candidate = wish.cache_candidates(valid)[0]
        self.assertEqual(candidate.params["authkey"], "synthetic-secret+value")
        self.assertNotIn("synthetic-secret", repr(candidate))
        self.assertFalse(wish.cache_candidates(valid.replace(b"webstatic-sea.hoyoverse.com", b"hoyoverse.com.attacker.invalid")))
        self.assertFalse(wish.cache_candidates(valid + b"&authkey=second"))

    def test_discovery_deduplicates_credentials_and_filters_regions(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            file = root / "webCaches/1/Cache/Cache_Data/data_2"
            file.parent.mkdir(parents=True)
            query = urlencode({"auth_appid": "webview_gacha", "authkey": "synthetic-secret", "game_biz": "hk4e_global", "region": REGION})
            url = ("https://webstatic.hoyoverse.com/?" + query).encode()
            file.write_bytes(url + b"\0" + url)
            candidates, files = wish.discover(root, REGION)
            self.assertEqual((len(candidates), files), (1, 1))
            self.assertEqual(wish.discover(root, "os_asia")[0], [])

    def test_pagination_preserves_large_ids_server_time_and_301_400_group(self):
        calls = []
        def request(context, banner, page, cursor):
            calls.append((banner, page, cursor))
            return {"region": REGION, "region_time_zone": "1", "list": [row("1700000000000000002"), row(banner="400")] if page == 1 else []}
        data = wish.fetch(access(), UID, REGION, request, lambda _: None, lambda _: None, ("301",))
        self.assertEqual(calls, [("301", 1, "0"), ("301", 2, "1700000000000000001")])
        self.assertEqual(data["records"][0]["id"], "1700000000000000001")
        self.assertEqual(data["records"][0]["time"], "2026-11-12 17:00:00")
        self.assertEqual(data["streams"][0]["region_time_zone"], 1)
        self.assertFalse(data["coverage"]["complete_lifetime_history"])
        self.assertTrue(data["coverage"]["all_available_pages_fetched"])
        self.assertNotIn("authkey", str(data))

    def test_wrong_uid_region_banner_empty_history_and_stuck_cursor_are_rejected(self):
        wrong_uid = row()
        wrong_uid["uid"] = "700000002"
        payloads = [
            {"list": [wrong_uid]},
            {"region": "os_asia", "list": [row()]},
            {"list": [row(banner="200")]},
            {"list": []},
            {"list": [row()]},
        ]
        for payload in payloads:
            with self.subTest(payload=payload), self.assertRaises(wish.ImportFailure):
                wish.fetch(access(), UID, REGION, lambda *_: payload, lambda _: None, lambda _: None, ("301",))

    def test_duplicate_conflicts_and_invalid_records_are_rejected(self):
        for key, value in [("id", "1.7e18"), ("count", "2"), ("time", "2026-02-30 00:00:00"), ("time", "2026-1-2 00:00:00"), ("rank_type", "99")]:
            raw = row()
            raw[key] = value
            with self.subTest(key=key), self.assertRaises(wish.ImportFailure):
                wish.record(raw, UID)
        def request(context, banner, page, cursor):
            return {"list": [row(banner=banner)] if page == 1 else []}
        with self.assertRaises(wish.ImportFailure):
            wish.fetch(access(), UID, REGION, request, lambda _: None, lambda _: None, ("100", "200"))

    def test_network_errors_do_not_expose_authenticated_url(self):
        with patch.object(wish.urllib.request, "build_opener") as build:
            build.return_value.open.side_effect = URLError("https://example.invalid/?authkey=synthetic-secret")
            with self.assertRaises(wish.ImportFailure) as raised:
                wish.request_page(access(), "301", 1, "0")
            self.assertNotIn("synthetic-secret", str(raised.exception))

    def test_failed_import_creates_no_archive_and_existing_file_is_not_overwritten(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "history.json"
            argv = ["probe", "--expected-uid", UID, "--expected-region", REGION, "--output", str(output)]
            with patch.object(wish.sys, "argv", argv), patch.object(wish, "find_game_data", return_value=Path(directory)), patch.object(wish, "discover", return_value=([access()], 1)), patch.object(wish, "select_access", return_value=(access(), ("301", {"list": [row()]}))), patch.object(wish, "fetch", side_effect=wish.ImportFailure("Network unavailable")) as fetch:
                with self.assertRaises(wish.ImportFailure):
                    wish.main()
                self.assertFalse(output.exists())
                output.write_text("existing history", encoding="utf-8")
                with self.assertRaises(wish.ImportFailure):
                    wish.main()
                self.assertEqual(fetch.call_count, 1)
                self.assertEqual(output.read_text(), "existing history")


if __name__ == "__main__":
    unittest.main()
