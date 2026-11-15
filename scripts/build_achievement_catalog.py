"""Build local achievement display metadata from an already downloaded pinned catalog."""
import argparse
import json
import re
from pathlib import Path


def display_text(text, progress=None):
    """Resolve the default masculine localization variant and safe display markup."""
    text = re.sub(r"\{M#([^{}]*)\}", r"\1", text)
    text = re.sub(r"\{F#[^{}]*\}", "", text)
    text = text.removeprefix("#")
    text = re.sub(r"</?(?:color|size|b|i)(?:=[^>]*)?>", "", text)
    if progress is not None:
        text = text.replace("{param0}", str(progress))
    return text.replace("\\n", "\n")


def build_records(achievements, goal_rows, texts):
    goals = {g.get("id", 0): g for g in goal_rows}
    source = {r["id"]: r for r in achievements}

    def chain(record):
        seen = set()
        while record.get("preStageAchievementId"):
            if record["id"] in seen:
                raise ValueError("Cyclic achievement stages")
            seen.add(record["id"])
            previous = source[record["preStageAchievementId"]]
            if previous.get("goalId", 0) != record.get("goalId", 0):
                raise ValueError("Achievement chain crosses categories")
            record = previous
        return record["id"], len(seen) + 1

    rows = {}
    for r in achievements:
        root, stage = chain(r)
        goal = goals.get(r.get("goalId", 0), {})
        rows[str(r["id"])] = {
            "name": display_text(texts.get(str(r["titleTextMapHash"]), f"Достижение ID {r['id']}")),
            "description": display_text(texts.get(str(r.get("descTextMapHash")), ""), r.get("progress")),
            "goal_id": r.get("goalId", 0),
            "category": display_text(texts.get(str(goal.get("nameTextMapHash")), "Без категории")),
            "category_order": goal.get("orderId", 999999),
            "order": r.get("orderId", r["id"]),
            "chain_order": source[root].get("orderId", root),
            "chain_id": str(root), "stage": stage,
            # Display exclusion only: raw captured facts remain untouched.
            "is_disused": r.get("isDisuse", False),
            "excluded": r.get("isDisuse", False) or r["id"] == 84517,
        }
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--output", type=Path, default=Path("local-data/achievements.ru.json"))
    args = parser.parse_args()
    if not re.fullmatch(r"[a-f0-9]{40}", args.revision):
        raise ValueError("Expected a pinned catalog revision")
    cache = Path(__file__).resolve().parent.parent / "local-data/catalog-cache" / args.revision
    def load(name):
        return json.loads((cache / name).read_text(encoding="utf-8-sig"))
    texts = {}
    for prefix in ("TextMapRU", "TextMap_MediumRU"):
        for part in range(2):
            texts.update(load(f"TextMap_{prefix}_{part}.json"))
    rows = build_records(load("ExcelBinOutput_AchievementExcelConfigData.json"), load("ExcelBinOutput_AchievementGoalExcelConfigData.json"), texts)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps({"schema_version": 1, "revision": args.revision, "records": rows}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Achievement display catalog: {len(rows)} records")


if __name__ == "__main__":
    main()
