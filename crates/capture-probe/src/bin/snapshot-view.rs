use anyhow::{Context, Result, bail};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashMap, fs, path::PathBuf};

#[derive(Parser)]
#[command(about = "Build an offline, searchable Russian inventory report from capture JSONL")]
struct Args {
    #[arg(default_value = "inventory.jsonl")]
    input: PathBuf,
    #[arg(long, default_value = "local-data/materials.ru.json")]
    catalog: PathBuf,
    #[arg(long, default_value = "inventory-view.html")]
    html: PathBuf,
}

#[derive(Deserialize)]
struct Catalog {
    schema_version: u32,
    language: String,
    source: String,
    revision: String,
    names: HashMap<String, String>,
    #[serde(default)]
    item_names: HashMap<String, String>,
    #[serde(default)]
    rarities: HashMap<String, u32>,
}

#[derive(Deserialize, Serialize)]
struct Material {
    item_id: u32,
    guid: String,
    count: u32,
}

fn latest_snapshot(text: &str) -> Result<Value> {
    let mut snapshot: Option<Value> = None;
    let mut changes = Vec::new();
    let mut final_state = Value::Null;
    let mut tracking_issues = 0;
    for (index, line) in text.trim_start_matches('\u{feff}').lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let event: Value = serde_json::from_str(line)
            .with_context(|| format!("Invalid JSONL at line {}", index + 1))?;
        if event["schema_version"] != 1 {
            bail!("Unsupported event schema at line {}", index + 1);
        }
        if event["event"] == "inventory_snapshot" {
            snapshot = Some(event);
            changes.clear();
            final_state = Value::Null;
            tracking_issues = 0;
        } else if let Some(current) = &snapshot {
            // Never attach another session's result to an older inventory snapshot.
            if event["session"] != current["session"] {
                final_state = Value::Null;
                continue;
            }
            match event["event"].as_str() {
                Some("material_changed") => {
                    final_state = Value::Null;
                    changes.push(json!({"change":event["data"]["change"],"source_elapsed_ms":event["source_elapsed_ms"]}));
                }
                Some("material_state") => final_state = event["data"].clone(),
                Some("material_update_ignored" | "material_update_rejected") => {
                    tracking_issues += 1
                }
                Some("status")
                    if matches!(
                        event["data"]["state"].as_str(),
                        Some("stale" | "waiting_for_snapshot")
                    ) =>
                {
                    final_state = Value::Null
                }
                _ => {}
            }
        }
    }
    let mut snapshot = snapshot.context("No inventory_snapshot found")?;
    snapshot["changes"] = json!(changes);
    snapshot["final_state"] = final_state;
    snapshot["tracking_issues"] = json!(tracking_issues);
    Ok(snapshot)
}

fn material_rows(snapshot: &Value, catalog: &Catalog) -> Result<(Vec<Value>, Vec<Value>, usize)> {
    let mut materials: Vec<Material> =
        serde_json::from_value(snapshot["data"]["materials"].clone())
            .context("Invalid snapshot materials")?;
    let tracked = snapshot["final_state"]["snapshot_available"] == true;
    let final_materials: Vec<Material> = if tracked {
        serde_json::from_value(snapshot["final_state"]["materials"].clone())
            .context("Invalid final material state")?
    } else {
        Vec::new()
    };
    let final_by_guid: HashMap<&str, &Material> = final_materials
        .iter()
        .map(|m| (m.guid.as_str(), m))
        .collect();
    let mut current: HashMap<String, u32> = materials
        .iter()
        .map(|m| (m.guid.clone(), m.count))
        .collect();
    let mut initial: HashMap<String, u32> = materials
        .iter()
        .map(|m| (m.guid.clone(), m.item_id))
        .collect();
    let mut history = Vec::new();
    let mut updates = HashMap::<String, usize>::new();
    for entry in snapshot["changes"].as_array().into_iter().flatten() {
        let change = &entry["change"];
        let guid = change["guid"].as_str().context("Invalid change GUID")?;
        let before = change["previous_count"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .context("Invalid previous count")?;
        let count = change["count"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .context("Invalid changed count")?;
        let delta = i64::from(count) - i64::from(before);
        anyhow::ensure!(
            change["removed"] != true || (count == 0 && change["new_entry"] != true),
            "Inconsistent deletion event"
        );
        if !initial.contains_key(guid) {
            let id = change["item_id"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| *n > 0)
                .context("Invalid new material ID")?;
            anyhow::ensure!(
                change["new_entry"] == true
                    && before == 0
                    && count > 0
                    && guid.parse::<u64>().is_ok_and(|n| n > 0),
                "Unmarked or invalid new material"
            );
            materials.push(Material {
                item_id: id,
                guid: guid.to_owned(),
                count: 0,
            });
            initial.insert(guid.to_owned(), id);
            current.insert(guid.to_owned(), 0);
        } else {
            anyhow::ensure!(change["new_entry"] != true, "Duplicate material creation");
        }
        let id = initial[guid];
        anyhow::ensure!(
            change["item_id"] == id && before == current[guid] && change["delta"] == delta,
            "Inconsistent material change history"
        );
        current.insert(guid.to_owned(), count);
        *updates.entry(guid.to_owned()).or_default() += 1;
        history.push(json!({"item_id":id,"guid":guid,"name":catalog.names.get(&id.to_string()),
            "previous_count":before,"count":count,"delta":delta,"source_elapsed_ms":entry["source_elapsed_ms"]}));
    }
    if tracked {
        anyhow::ensure!(
            final_by_guid.len() == materials.len() && final_materials.len() == materials.len(),
            "Final material set differs from snapshot"
        );
    }
    let mut unknown = 0;
    let rows = materials.iter().map(|m| {
        let name=catalog.names.get(&m.item_id.to_string()); if name.is_none(){ unknown+=1; }
        let count = if tracked {
            let final_m=final_by_guid.get(m.guid.as_str()).context("Missing final material")?;
            anyhow::ensure!(final_m.item_id==m.item_id && final_m.count==current[m.guid.as_str()], "Final count does not match change history");
            Some(final_m.count)
        } else { None };
        Ok(json!({"item_id":m.item_id,"guid":m.guid,"name":name,"initial_count":m.count,"count":count,
            "delta":count.map(|n|i64::from(n)-i64::from(m.count)),"updates":updates.get(m.guid.as_str()).copied().unwrap_or(0)}))
    }).collect::<Result<Vec<_>>>()?;
    Ok((rows, history, unknown))
}

fn signed(value: &Value) -> String {
    value
        .as_i64()
        .map(|n| format!("{n:+}"))
        .unwrap_or("—".into())
}

fn elapsed(value: &Value) -> String {
    value
        .as_u64()
        .map(|n| format!("{}:{:02}.{:03}", n / 60000, (n / 1000) % 60, n % 1000))
        .unwrap_or("—".into())
}

fn decode_capture(bytes: Vec<u8>) -> Result<String> {
    // Windows PowerShell 5 redirects native stdout to UTF-16LE by default.
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        let little_endian = bytes[0] == 0xff;
        if !(bytes.len() - 2).is_multiple_of(2) {
            bail!("Truncated UTF-16 capture");
        }
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|chunk| {
                if little_endian {
                    u16::from_le_bytes([chunk[0], chunk[1]])
                } else {
                    u16::from_be_bytes([chunk[0], chunk[1]])
                }
            })
            .collect();
        return String::from_utf16(&units).context("Invalid UTF-16 capture");
    }
    String::from_utf8(bytes).context("Invalid UTF-8 capture")
}

fn script_json(value: &Value) -> Result<String> {
    Ok(serde_json::to_string(value)?
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e"))
}

fn html_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn render_report(report: &Value) -> Result<String> {
    let rows = report["rows"].as_array().context("Missing report rows")?;
    let mut sorted: Vec<&Value> = rows.iter().collect();
    sorted.sort_by_key(|row| row["item_id"].as_u64().unwrap_or_default());
    let table: String = sorted
        .into_iter()
        .map(|row| {
            format!(
                "<tr><td>{}</td><td class=\"id\">{}</td><td>{}</td><td>{}</td><td>{}</td></tr>\n",
                html_text(row["name"].as_str().unwrap_or("Неизвестный материал")),
                row["item_id"],
                row.get("initial_count").unwrap_or(&row["count"]),
                row["count"]
                    .as_u64()
                    .map(|n| n.to_string())
                    .unwrap_or("—".into()),
                signed(&row["delta"])
            )
        })
        .collect();
    let summary = format!(
        "{} записей материалов · {} без названия · {}",
        rows.len(),
        report["unknown"],
        if report["tracked"] == true {
            "учтены изменения из записи"
        } else {
            "только исходный снимок"
        }
    );
    let history = report["history"].as_array().cloned().unwrap_or_default();
    let history_table: String = history.iter().map(|row|format!(
        "<tr><td>{}</td><td>{}</td><td class=\"id\">{}</td><td>{}</td><td>{}</td><td>{}</td></tr>\n",
        elapsed(&row["source_elapsed_ms"]),html_text(row["name"].as_str().unwrap_or("Неизвестный материал")),row["item_id"],row["previous_count"],row["count"],signed(&row["delta"])
    )).collect();
    let notice = if report["tracked"] == true {
        "Итог рассчитан по сохранённой записи, включая появление и полное расходование материалов. Нулевые записи сохранены для сравнения. Предметы показаны по исходному снимку."
    } else {
        "Доступен исходный снимок; итоговое состояние материалов отсутствует. Для расчёта нужен импорт с --track-materials. Отсутствие ресурса в таблице не означает нулевой запас."
    };
    let item_rows = report["item_rows"].as_array().cloned().unwrap_or_default();
    let item_table:String=item_rows.iter().map(|row| {
        format!("<tr><td>{}<br><small class=\"id\">{}</small></td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>\n",
            html_text(row["name"].as_str().unwrap_or("Неизвестный предмет")),
            html_text(&format!("ID {} · {}",row["item_id"],row["guid"].as_str().unwrap_or(""))),
            html_text(row["category"].as_str().unwrap_or("Неизвестно")),
            html_text(row["level_label"].as_str().unwrap_or("—")),
            html_text(row["details"].as_str().unwrap_or("—")),
            html_text(&row["count"].as_u64().map(|n|n.to_string()).unwrap_or("—".into())))
    }).collect();
    let item_summary = if report["items_available"] == true {
        format!(
            "{} записей предметов · {} без названия. Одинаковые экземпляры сохранены отдельно.",
            item_rows.len(),
            report["item_unknown"]
        )
    } else {
        "В этом старом снимке предметы не сохранялись. Для их просмотра нужно повторно разобрать исходную запись новой версией приложения.".into()
    };
    let source = format!(
        "Названия: {} · {} · ревизия {}. Отчёт работает локально.",
        report["catalog"]["language"].as_str().unwrap_or("?"),
        report["catalog"]["source"].as_str().unwrap_or("?"),
        report["catalog"]["revision"].as_str().unwrap_or("?")
    );
    // Render content on disk; JavaScript only enhances filtering and sorting.
    // Insert JSON last so user data cannot collide with template markers.
    Ok(include_str!("../../assets/snapshot-tabs.html")
        .replace("<!--__SUMMARY__-->", &html_text(&summary))
        .replace("<!--__NOTICE__-->", &html_text(&format!("{}{}",if report["synthetic"]==true {"ДЕМОНСТРАЦИОННЫЕ ДАННЫЕ. "} else {""},notice)))
        .replace("<!--__HISTORY_ROWS__-->", &history_table)
        .replace("<!--__HISTORY_COUNT__-->", &history.len().to_string())
        .replace("<!--__HISTORY_SUMMARY__-->", &html_text(&format!("{} изменений после выбранного снимка. Время от начала записи. Пропущенных или отклонённых сообщений обновления: {}.",history.len(),report["tracking_issues"].as_u64().unwrap_or(0))))
        .replace("<!--__ROWS__-->", &table)
        .replace("<!--__ITEM_ROWS__-->", &item_table)
        .replace("<!--__ITEM_SUMMARY__-->", &html_text(&item_summary))
        .replace("<!--__MATERIAL_COUNT__-->", &rows.len().to_string())
        .replace("<!--__ITEM_COUNT__-->", &item_rows.len().to_string())
        .replace("<!--__SOURCE__-->", &html_text(&source))
        .replace("/*__REPORT__*/null", &script_json(report)?))
}

fn item_rows(snapshot: &Value, catalog: &Catalog) -> Result<(Vec<Value>, usize)> {
    let Some(items) = snapshot["data"]["items"].as_array() else {
        return Ok((Vec::new(), 0));
    };
    let mut unknown = 0;
    let rows=items.iter().map(|item| {
        let id=item["item_id"].as_u64().context("Invalid item ID")?;
        let guid=item["guid"].as_str().context("Invalid item GUID")?;
        let kind=item["kind"].as_str().context("Invalid item category")?;
        let name=catalog.item_names.get(&id.to_string()).or_else(||catalog.names.get(&id.to_string()));if name.is_none(){unknown+=1;}
        let category=match kind {"weapon"=>"Оружие","artifact"=>"Артефакт","furniture"=>"Мебель","beyond_material"=>"Материал Астрального предела","beyond_facility"=>"Декор Астрального предела","tps_weapon"=>"TPS-оружие",_=>"Другое"};
        let mut details=Vec::new();
        if let Some(rarity)=catalog.rarities.get(&id.to_string()){details.push(format!("{rarity}★"));}
        if let Some(ascension)=item["ascension"].as_u64(){details.push(format!("Возвышение {ascension}"));}
        if let Some(refinement)=item["refinement"].as_u64(){details.push(format!("Пробуждение {refinement}"));}
        if item["locked"]==true {details.push("Заблокировано".into());}
        if let Some(ids)=item["accessory_ids"].as_array(){details.push(format!("Аксессуары: {}",ids.iter().map(Value::to_string).collect::<Vec<_>>().join(", ")));}
        let level=item["level"].as_u64().map(|n|if kind=="artifact" {format!("+{n}")} else {n.to_string()}).unwrap_or("—".into());
        Ok(json!({"item_id":id,"guid":guid,"name":name,"kind":kind,"category":category,
            "level":item["level"],"level_label":level,"details":details.join(" · "),"count":item["count"]}))
    }).collect::<Result<Vec<_>>>()?;
    Ok((rows, unknown))
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.html.extension().and_then(|s| s.to_str()) != Some("html") {
        bail!("Output must have .html extension");
    }
    let snapshot = latest_snapshot(&decode_capture(
        fs::read(&args.input).context("Cannot read capture JSONL")?,
    )?)?;
    let catalog: Catalog = serde_json::from_slice(
        &fs::read(&args.catalog)
            .context("Cannot read catalog; run scripts/Update-MaterialCatalog.ps1 first")?,
    )?;
    if catalog.schema_version != 1 {
        bail!("Unsupported catalog schema");
    }
    let (rows, history, unknown) = material_rows(&snapshot, &catalog)?;
    let (item_rows, item_unknown) = item_rows(&snapshot, &catalog)?;
    let report = json!({
        "rows": rows, "unknown": unknown, "synthetic": snapshot["data"]["synthetic"],
        "history":history,"tracked":snapshot["final_state"]["snapshot_available"]==true,"tracking_issues":snapshot["tracking_issues"],
        "item_rows":item_rows,"item_unknown":item_unknown,"items_available":snapshot["data"]["items"].is_array(),
        "observed_unix_ms": snapshot["observed_unix_ms"], "session": snapshot["session"],
        "catalog": {"language": catalog.language, "source": catalog.source, "revision": catalog.revision},
    });
    let html = render_report(&report)?;
    fs::write(&args.html, html).context("Cannot write HTML report")?;
    println!(
        "{} materials; {} names resolved; {} unknown. {} item instances ({} unnamed). Report: {}",
        rows.len(),
        rows.len() - unknown,
        unknown,
        item_rows.len(),
        item_unknown,
        args.html.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Catalog, Vec<Value>) {
        let catalog = serde_json::from_value(json!({"schema_version":1,"language":"ru","source":"test","revision":"test","names":{"100011":"Гриб <script>"}})).unwrap();
        let events = vec![
            json!({"schema_version":1,"event":"inventory_snapshot","session":"2","data":{"materials":[{"item_id":100011,"guid":"18446744073709551615","count":315}]}}),
            json!({"schema_version":1,"event":"material_changed","session":"2","source_elapsed_ms":25449,"data":{"change":{"item_id":100011,"guid":"18446744073709551615","previous_count":315,"count":316,"delta":1}}}),
            json!({"schema_version":1,"event":"material_changed","session":"2","source_elapsed_ms":35009,"data":{"change":{"item_id":100011,"guid":"18446744073709551615","previous_count":316,"count":315,"delta":-1}}}),
            json!({"schema_version":1,"event":"material_state","session":"2","data":{"snapshot_available":true,"materials":[{"item_id":100011,"guid":"18446744073709551615","count":315}]}}),
        ];
        (catalog, events)
    }
    fn parsed(events: &[Value]) -> Value {
        latest_snapshot(
            &events
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap()
    }
    #[test]
    fn renders_history_and_keeps_changes_even_when_net_difference_is_zero() {
        let (catalog, events) = fixture();
        let snapshot = parsed(&events);
        let (rows, history, unknown) = material_rows(&snapshot, &catalog).unwrap();
        assert_eq!(unknown, 0);
        assert_eq!(rows[0]["initial_count"], 315);
        assert_eq!(rows[0]["count"], 315);
        assert_eq!(rows[0]["delta"], 0);
        assert_eq!(rows[0]["updates"], 2);
        assert_eq!(history.len(), 2);
        let html = render_report(
            &json!({"rows":rows,"history":history,"tracked":true,"unknown":0,"catalog":{}}),
        )
        .unwrap();
        let static_html = html.split("<script>").next().unwrap();
        assert!(static_html.contains("0:25.449"));
        assert!(static_html.contains("0:35.009"));
        assert!(static_html.contains("Гриб &lt;script&gt;"));
        assert!(static_html.contains("<td>+1</td>"));
        assert!(static_html.contains("<td>-1</td>"));
        assert!(!static_html.contains("<!--__"));
    }
    #[test]
    fn incomplete_or_other_session_state_never_supplies_final_count() {
        let (catalog, mut events) = fixture();
        let (rows, history, _) = material_rows(&parsed(&events[..3]), &catalog).unwrap();
        assert!(rows[0]["count"].is_null());
        assert_eq!(history.len(), 2);
        events[3]["session"] = json!("3");
        assert!(material_rows(&parsed(&events), &catalog).unwrap().0[0]["count"].is_null());
        events.push(json!({"schema_version":1,"event":"inventory_snapshot","session":"3","data":{"materials":[{"item_id":100011,"guid":"new","count":42}]}}));
        let (rows, history, _) = material_rows(&parsed(&events), &catalog).unwrap();
        assert_eq!(rows[0]["initial_count"], 42);
        assert!(history.is_empty());
    }
    #[test]
    fn rejects_inconsistent_history_or_final_state() {
        let (catalog, mut events) = fixture();
        events[3]["data"]["materials"][0]["count"] = json!(314);
        assert!(material_rows(&parsed(&events), &catalog).is_err());
        events[3]["data"]["materials"][0]["count"] = json!(315);
        events[1]["data"]["change"]["previous_count"] = json!(314);
        assert!(material_rows(&parsed(&events), &catalog).is_err());
    }
    #[test]
    fn includes_created_material_and_rejects_unmarked_creation() {
        let (catalog, mut events) = fixture();
        events.insert(3,json!({"schema_version":1,"event":"material_changed","session":"2","source_elapsed_ms":99834,
            "data":{"change":{"item_id":113082,"guid":"12345678901234567890","previous_count":0,"count":1,"delta":1,"new_entry":true}}}));
        events[4]["data"]["materials"]
            .as_array_mut()
            .unwrap()
            .push(json!({"item_id":113082,"guid":"12345678901234567890","count":1}));
        let (rows, history, unknown) = material_rows(&parsed(&events), &catalog).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(history.len(), 3);
        assert_eq!(unknown, 1);
        let created = rows.iter().find(|row| row["item_id"] == 113082).unwrap();
        assert_eq!(created["initial_count"], 0);
        assert_eq!(created["count"], 1);
        assert_eq!(created["updates"], 1);
        assert_eq!(created["delta"], 1);
        events[3]["data"]["change"]["new_entry"] = json!(false);
        assert!(material_rows(&parsed(&events), &catalog).is_err());
    }
    #[test]
    fn item_tab_preserves_instances_and_renders_without_javascript() {
        let catalog:Catalog=serde_json::from_value(json!({"schema_version":1,"language":"ru","source":"local","revision":"test","names":{},"item_names":{"11101":"Меч <script>"},"rarities":{"11101":5}})).unwrap();
        let snapshot = json!({"data":{"items":[
            {"item_id":11101,"guid":"18446744073709551615","kind":"weapon","level":90,"ascension":6,"refinement":5,"locked":true,"count":1},
            {"item_id":11101,"guid":"18446744073709551614","kind":"weapon","level":1,"count":1}
        ]}});
        let (rows, unknown) = item_rows(&snapshot, &catalog).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(unknown, 0);
        assert!(
            rows[0]["details"]
                .as_str()
                .unwrap()
                .contains("Пробуждение 5")
        );
        let html=render_report(&json!({"rows":[],"unknown":0,"catalog":{},"item_rows":rows,"items_available":true,"item_unknown":0})).unwrap();
        let static_html = html.split("<script>").next().unwrap();
        assert!(static_html.contains("Меч &lt;script&gt;"));
        assert!(static_html.contains("18446744073709551615"));
        assert!(static_html.contains("18446744073709551614"));
        assert!(static_html.contains("id=\"tab-items\""));
        assert!(static_html.contains("2 записей предметов"));
        let old = render_report(&json!({"rows":[],"unknown":0,"catalog":{}})).unwrap();
        assert!(old.contains("предметы не сохранялись"));
    }
    #[test]
    fn report_contains_visible_rows_without_running_javascript() {
        let report = json!({"rows": [{"item_id": 100051, "count": 621, "name": "Ягода <script>"}], "unknown": 0, "catalog": {}});
        let html = render_report(&report).unwrap();
        let static_part = html.split("<script>").next().unwrap();
        assert!(static_part.contains("Ягода &lt;script&gt;"));
        assert!(static_part.contains("<td>621</td>"));
        assert!(static_part.contains("1 записей"));
        assert!(!static_part.contains("<!--__ROWS__-->"));
    }
    #[test]
    fn reads_powershell_utf16_and_utf8_without_changing_names() {
        let text = "{\"name\":\"Ягода\"}";
        let bytes: Vec<u8> = [0xff, 0xfe]
            .into_iter()
            .chain(text.encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        assert_eq!(decode_capture(bytes).unwrap(), text);
        assert_eq!(decode_capture(text.as_bytes().to_vec()).unwrap(), text);
        assert!(decode_capture(vec![0xff, 0xfe, 0]).is_err());
    }
    #[test]
    fn selects_latest_snapshot_and_rejects_truncated_input() {
        let text = "\u{feff}{\"schema_version\":1,\"event\":\"inventory_snapshot\",\"session\":\"1\"}\n{\"schema_version\":1,\"event\":\"inventory_snapshot\",\"session\":\"2\"}\n{\"schema_version\":1,\"event\":\"status\"}";
        assert_eq!(latest_snapshot(text).unwrap()["session"], "2");
        assert!(latest_snapshot(&(text.to_owned() + "\n{bad")).is_err());
        assert!(latest_snapshot("{\"schema_version\":1,\"event\":\"status\"}").is_err());
    }
    #[test]
    fn embedded_data_cannot_close_script_tag() {
        let text = script_json(&json!({"name": "</script><script>alert('x')</script>&"})).unwrap();
        assert!(!text.contains('<'));
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["name"], "</script><script>alert('x')</script>&");
    }
}
