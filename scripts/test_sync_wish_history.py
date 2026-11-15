import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
import sync_wish_history as sync
import import_wish_history as wish


class SyncTests(unittest.TestCase):
    def test_cached_verified_first_page_is_reused_and_secrets_are_not_serialized(self):
        candidate=wish.Access({"authkey":"private-secret","region":"os_euro"},0,0,0)
        row={"id":"12345678901234567890","uid":"700000001","gacha_type":"301","item_id":"","count":"1","time":"2026-11-15 01:00:00","name":"Меч","item_type":"Оружие","rank_type":"3","lang":"ru"}
        first={"list":[row],"region":"os_euro","region_time_zone":1}
        with tempfile.TemporaryDirectory() as d, patch.object(wish,"find_game_data",return_value=Path(d)), patch.object(wish,"discover",return_value=([candidate],1)), patch.object(wish,"select_access",return_value=(candidate,("301",first))), patch.object(wish,"request_page",return_value={"list":[],"region":"os_euro","region_time_zone":1}) as request, patch.object(wish.time,"sleep"):
            target=Path(d)/"history.json";sync.synchronize("700000001","os_euro",target)
            data=json.loads(target.read_text(encoding="utf-8"))
            self.assertEqual(len(data["records"]),1)
            self.assertEqual(len(data["streams"]),5)
            self.assertTrue(data["coverage"]["all_available_pages_fetched"])
            self.assertNotIn("private-secret",target.read_text(encoding="utf-8"))
            self.assertFalse(any(call.args[1:]==("301",1,"0") for call in request.call_args_list))

    def test_network_failure_leaves_no_partial_archive(self):
        with tempfile.TemporaryDirectory() as d, patch.object(wish,"find_game_data",return_value=Path(d)), patch.object(wish,"discover",return_value=([object()],1)), patch.object(wish,"select_access",side_effect=wish.ImportFailure("Cannot connect")):
            target=Path(d)/"history.json"
            with self.assertRaises(wish.ImportFailure):sync.synchronize("700000001","os_euro",target)
            self.assertFalse(target.exists())


if __name__=="__main__":unittest.main()
