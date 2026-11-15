"""Build a standalone, filterable HTML report from achievement research findings."""
import argparse
from collections import Counter
from datetime import datetime, timezone
from html import escape
import json
from pathlib import Path
import re


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("--html", required=True, type=Path)
    parser.add_argument("--catalog-cache", type=Path)
    args = parser.parse_args()
    if args.input.suffix.lower() == ".jsonl":
        catalog = json.loads((Path(__file__).resolve().parent.parent / "local-data/achievements.ru.json").read_text(encoding="utf-8-sig"))
        records = {}
        session = None
        for line in args.input.read_text(encoding="utf-8-sig").splitlines():
            if not line.strip():
                continue
            event = json.loads(line)
            if event["event"] == "achievement_snapshot":
                session = event["session"]
                records = {r["achievement_id"]: r for r in event["data"]["records"]}
            elif event["event"] == "achievement_changed":
                change = event["data"]
                if event["session"] != session or records.get(change["achievement_id"]) != change["previous"]:
                    raise ValueError("Inconsistent achievement event history")
                records[change["achievement_id"]] = change["current"]
        if session is None:
            raise ValueError("No normalized achievement snapshot")
        data = {"catalog_revision": catalog["revision"], "records": [{**row, **catalog["records"].get(str(id), {"name": f"Достижение ID {id}", "description": "", "goal_id": 0})} for id, row in records.items()]}
    else:
        data = json.loads(args.input.read_text(encoding="utf-8-sig"))
    revision = data["catalog_revision"]
    if not re.fullmatch(r"[a-f0-9]{40}", revision):
        raise ValueError("Expected a pinned catalog revision")
    root = Path(__file__).resolve().parent.parent
    cache = args.catalog_cache or root / "local-data" / "catalog-cache" / revision
    texts = {}
    for prefix in ("TextMapRU", "TextMap_MediumRU"):
        for part in range(2):
            texts.update(json.loads((cache / f"TextMap_{prefix}_{part}.json").read_text(encoding="utf-8-sig")))
    goals = {}
    for goal in json.loads((cache / "ExcelBinOutput_AchievementGoalExcelConfigData.json").read_text(encoding="utf-8-sig")):
        goal_id = goal.get("id", 0)  # Omitted numeric IDs in this table default to zero.
        goals[goal_id] = (texts.get(str(goal["nameTextMapHash"]), f"Раздел ID {goal_id}"), goal.get("orderId", 0))
    records = data["records"]
    categories = Counter(int(r.get("goal_id") or 0) for r in records)
    statuses = Counter(r["status_code"] for r in records)
    labels = {1: "Не выполнено", 2: "Выполнено · награда не получена", 3: "Награда получена"}
    options = ['<option value="all">Все категории</option>']
    for goal_id in sorted(categories, key=lambda key: (goals.get(key, ("", 999999))[1], key)):
        name = goals.get(goal_id, (f"Раздел ID {goal_id}", 0))[0]
        options.append(f'<option value="{goal_id}">{escape(name)} ({categories[goal_id]})</option>')
    rows = []
    for record in sorted(records, key=lambda r: (r["status_code"] in (2, 3), r["achievement_id"])):
        goal_id = int(record.get("goal_id") or 0)
        category = goals.get(goal_id, (f"Раздел ID {goal_id}", 0))[0]
        status = record["status_code"]
        current = record["current_progress"]
        if current is None and status in (2, 3):
            current = record["total_progress"]
        progress = "—" if current is None else str(current)
        progress += f' / {record["total_progress"]}'
        date = datetime.fromtimestamp(record["finish_unix"], timezone.utc).strftime("%Y-%m-%d %H:%M:%S UTC") if record["finish_unix"] else "—"
        cells = [record["achievement_id"], record["name"], category, labels.get(status, f"Статус {status}"), progress, date, record["description"] or ""]
        rows.append(f'<tr data-category="{goal_id}" data-status="{status}">' + "".join(f"<td>{escape(str(value))}</td>" for value in cells) + "</tr>")
    values = {
        "ROWS": "".join(rows), "CATEGORIES": "".join(options),
        "TOTAL": str(len(records)), "COMPLETED": str(statuses[2] + statuses[3]),
        "UNFINISHED": str(statuses[1]), "UNCLAIMED": str(statuses[2]), "CLAIMED": str(statuses[3]),
    }
    template = (root / "crates" / "capture-probe" / "assets" / "achievement-report.html").read_text(encoding="utf-8")
    rendered = re.sub(r"__REPORT_(\w+)__", lambda match: values[match[1]], template)
    args.html.write_text(rendered, encoding="utf-8")
    print(f"Saved {len(records)} achievements, {len(categories)} categories: {args.html}")


if __name__ == "__main__":
    main()
