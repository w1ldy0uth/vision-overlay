"""Render an offline diagnostic page from a normalized wish-history import."""
import argparse
from collections import Counter
from datetime import datetime, timedelta, timezone
from html import escape
import json
from pathlib import Path
import re

from import_wish_history import REGIONS, record


LABELS = {"100": "Молитва новичка", "200": "Стандартная", "301": "Событие персонажа · I", "400": "Событие персонажа · II", "302": "Событие оружия", "500": "Молитва хроник"}
GROUPS = (("all", "Все молитвы"), ("characters", "Персонажи"), ("302", "Оружие"), ("200", "Стандартные"), ("100", "Новичка"), ("500", "Хроник"))


def build(data, source_name):
    if data.get("schema_version") != 1 or data.get("server_region") not in REGIONS:
        raise ValueError("Unsupported wish archive")
    uid = data.get("uid")
    if not isinstance(uid, str) or not re.fullmatch(r"[1-9][0-9]{5,19}", uid):
        raise ValueError("Invalid profile UID")
    rows = [record(row, uid) for row in data["records"]]
    if len({row["id"] for row in rows}) != len(rows):
        raise ValueError("Duplicate wish IDs in archive")
    # Python integers preserve all ID digits; the browser only uses this small ordinal.
    rows.sort(key=lambda row: int(row["id"]))
    counts = Counter(row["gacha_type"] for row in rows)
    rarities = Counter(row["rank_type"] for row in rows)
    missing_ids = sum(not row["item_id"] for row in rows)
    regions = {"os_euro": "Европа", "os_asia": "Азия", "os_usa": "Северная Америка", "os_cht": "Тайвань / Гонконг"}
    group_counts = {**counts, "all": len(rows), "characters": counts["301"] + counts["400"]}
    buttons = "".join(f'<button class="banner" data-banner="{key}" aria-pressed="{str(key == "all").lower()}"><span>{label}</span><strong>{group_counts.get(key, 0)}</strong></button>' for key, label in GROUPS)
    options = '<option value="all">Все баннеры</option><option value="characters">События персонажей · I + II</option>' + "".join(f'<option value="{key}">{label} ({counts[key]})</option>' for key, label in LABELS.items())
    types = sorted({row["item_type"] or "Не указан" for row in rows})
    type_options = '<option value="all">Любой тип результата</option>' + "".join(f'<option value="{escape(value, quote=True)}">{escape(value)}</option>' for value in types)
    table_rows = []
    for index, row in reversed(list(enumerate(rows))):
        banner = row["gacha_type"]
        kind = row["item_type"] or "Не указан"
        search = " ".join(str(row[key] or "") for key in ("name", "id", "item_id"))
        raw = escape(json.dumps(row, ensure_ascii=False, indent=2))
        table_rows.append(
            f'<tr data-order="{index}" data-banner="{banner}" data-rarity="{row["rank_type"]}" data-kind="{escape(kind, quote=True)}" data-date="{row["time"][:10]}" data-search="{escape(search, quote=True)}">'
            f'<td class="date">{escape(row["time"])}</td><td class="result"><strong>{escape(row["name"])}</strong>'
            f'<details><summary>Сырые данные</summary><pre>{raw}</pre></details></td>'
            f'<td><span class="rarity r{row["rank_type"]}">{row["rank_type"]} ★</span></td><td>{escape(kind)}</td>'
            f'<td>{LABELS[banner]}<small>Тип {banner}</small></td><td class="mono">{escape(row["id"])}</td>'
            f'<td class="mono">{escape(row["item_id"] or "Не передан")}</td></tr>'
        )
    streams = "".join(f'<tr><td>{LABELS[str(stream["gacha_type"])]}</td><td>{escape(str(stream["pages"]))}</td><td>{escape(str(stream.get("region_from_api") or "Не указан"))}</td></tr>' for stream in data.get("streams", []))
    times = [row["time"] for row in rows]
    coverage = data.get("coverage", {})
    complete = coverage.get("all_available_pages_fetched") is True
    period = f'{min(times)[:10]} — {max(times)[:10]}' if times else "Записей нет"
    zone = data.get("region_time_zone")
    zone_label = "API не указал часовой пояс" if zone is None else f'Часовой пояс API: UTC{int(zone):+d}'
    imported = datetime.fromisoformat(data["fetched_at"]).astimezone(timezone(timedelta(hours=3))).strftime("%d.%m.%Y · %H:%M МСК")
    values = {
        "UID": escape(uid), "REGION": escape(regions[data["server_region"]]), "REGION_CODE": escape(data["server_region"]),
        "REGION_PROOF": "Подтверждён API" if data.get("region_evidence") == "api_response" else "Выбран при импорте",
        "IMPORTED": imported, "TOTAL": str(len(rows)), "FIVE": str(rarities["5"]), "FOUR": str(rarities["4"]), "THREE": str(rarities["3"]),
        "PERIOD": period, "BANNERS": buttons, "OPTIONS": options, "TYPES": type_options, "ROWS": "".join(table_rows), "STREAMS": streams,
        "SOURCE": escape(source_name), "MISSING": str(missing_ids), "ZONE": escape(zone_label),
        "COVERAGE": "Все доступные страницы загружены" if complete else "Загрузка доступных страниц не завершена",
        "LIFETIME": "Полнота за всё время отмечена источником" if coverage.get("complete_lifetime_history") is True else "Полнота за всё время аккаунта не подтверждена",
    }
    template = (Path(__file__).resolve().parent.parent / "crates/capture-probe/assets/wish-report.html").read_text(encoding="utf-8")
    return re.sub(r"__WISH_(\w+)__", lambda match: values[match[1]], template)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("--html", type=Path, required=True)
    args = parser.parse_args()
    data = json.loads(args.input.read_text(encoding="utf-8-sig"))
    rendered = build(data, args.input.name)
    args.html.write_text(rendered, encoding="utf-8")
    print(f"Wish report: {len(data['records'])} records, UID {data['uid']}: {args.html}")


if __name__ == "__main__":
    main()
