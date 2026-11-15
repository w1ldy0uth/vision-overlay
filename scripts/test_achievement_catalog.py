import unittest
from build_achievement_catalog import build_records, display_text


class AchievementCatalogTests(unittest.TestCase):
    def test_localization_markup_and_threshold(self):
        self.assertEqual(display_text("#{M#Хозяин}{F#Хозяйка} зоопарка"), "Хозяин зоопарка")
        self.assertEqual(display_text("#Ты думал{F#a}, это всерьёз?"), "Ты думал, это всерьёз?")
        self.assertEqual(display_text("<color=#fff>Получите {param0}</color>", 30), "Получите 30")

    def test_protocol_chain_and_display_exclusion(self):
        records = [dict(id=1, titleTextMapHash=1, orderId=4),
                   dict(id=2, titleTextMapHash=1, preStageAchievementId=1, orderId=5),
                   dict(id=84517, titleTextMapHash=1)]
        rows = build_records(records, [dict(nameTextMapHash=2, orderId=1)], {"1":"Название", "2":"Категория"})
        self.assertEqual(rows["2"]["chain_id"], "1")
        self.assertEqual(rows["2"]["stage"], 2)
        self.assertEqual(rows["2"]["chain_order"], 4)
        self.assertTrue(rows["84517"]["excluded"])
        self.assertFalse(rows["1"]["excluded"])
        records[0]["preStageAchievementId"] = 2
        with self.assertRaisesRegex(ValueError, "Cyclic"):
            build_records(records, [], {})


if __name__ == "__main__":
    unittest.main()
