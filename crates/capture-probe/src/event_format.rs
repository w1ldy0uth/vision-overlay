//! Shared console/panel presentation. Identity and raw values remain in JSONL.
use serde_json::Value;
use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};

pub fn timestamp(unix_ms: u64) -> anyhow::Result<String> {
    let utc = OffsetDateTime::from_unix_timestamp_nanos(i128::from(unix_ms) * 1_000_000)?;
    let offset = UtcOffset::local_offset_at(utc).unwrap_or(UtcOffset::UTC);
    Ok(utc.to_offset(offset).format(&Rfc3339)?)
}
pub fn event_timestamp(event: &Value) -> Option<String> {
    event["timestamp"].as_str().map(str::to_owned).or_else(|| {
        let ms = event["observed_unix_ms"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .or_else(|| event["observed_unix_ms"].as_u64())?;
        timestamp(ms).ok()
    })
}
pub fn item_description(item: &Value) -> String {
    if item.is_null() {
        return "—".into();
    }
    match item["kind"].as_str() {
        Some("artifact" | "weapon") => {
            let mut description = format!(
                "ур. {} · опыт {}",
                item["level"],
                item["exp"]
                    .as_u64()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "—".into())
            );
            if item["kind"] == "weapon" {
                if let Some(ascension) = item["ascension"].as_u64() {
                    description.push_str(&format!(" · возв. {ascension}"));
                }
                if let Some(refinement) = item["refinement"].as_u64() {
                    description.push_str(&format!(" · R{refinement}"));
                }
            }
            description
        }
        _ => item["count"]
            .as_u64()
            .map(|v| format!("{v} шт."))
            .unwrap_or_else(|| "Есть".into()),
    }
}
pub fn item_action(change: &Value) -> String {
    if change["removed"] == true {
        return "Удалён".into();
    }
    if change["new_entry"] == true {
        return "Получен".into();
    }
    let before = &change["previous"];
    let after = &change["item"];
    let fields = [
        ("level", "Уровень"),
        ("exp", "Опыт"),
        ("ascension", "Возвышение"),
        ("refinement", "Пробуждение"),
        ("locked", "Блокировка"),
        ("starred", "Избранное"),
        ("main_prop_id", "Основной стат"),
        ("append_prop_ids", "Сабстаты"),
        ("count", "Количество"),
    ];
    let changes: Vec<_> = fields
        .iter()
        .filter(|(key, _)| before[*key] != after[*key])
        .map(|(_, label)| *label)
        .collect();
    if changes.is_empty() {
        "Обновлён".into()
    } else {
        format!("Изменено: {}", changes.join(", "))
    }
}

/// Missing progress stays missing in data. Completion supplies a display lower bound.
pub fn achievement_display(row: &Value) -> String {
    if row.is_null() {
        return "—".into();
    }
    let status = match row["status_code"].as_u64() {
        Some(1) => "Не выполнено",
        Some(2) => "Выполнено · награда не получена",
        Some(3) => "Награда получена",
        _ => "Неизвестный статус",
    };
    let progress = row["current_progress"].as_u64().or_else(|| {
        if matches!(row["status_code"].as_u64(), Some(2 | 3)) {
            row["total_progress"].as_u64()
        } else {
            None
        }
    });
    format!(
        "{} / {} · {status}",
        progress
            .map(|n| n.to_string())
            .unwrap_or_else(|| "—".into()),
        row["total_progress"]
    )
}
pub fn achievement_action(data: &Value) -> &'static str {
    match data["action"].as_str() {
        Some("completed") => "Достижение выполнено",
        Some("reward_claimed") => "Награда за достижение получена",
        _ => "Прогресс достижения",
    }
}

pub fn adventure_display(row: &Value) -> String {
    if row.is_null() {
        return "—".into();
    }
    format!(
        "Ранг {} · {} опыта",
        row["rank"],
        row["exp"]
            .as_u64()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "—".into())
    )
}
pub fn adventure_action(data: &Value) -> String {
    if data["previous"]["rank"] != data["current"]["rank"] {
        "Повышение ранга приключений".into()
    } else {
        data["exp_delta"]
            .as_u64()
            .map(|n| format!("Опыт приключений: +{n}"))
            .unwrap_or_else(|| "Опыт приключений обновлён".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn weapon_refinement_and_ascension_are_visible_even_without_level_or_exp_changes() {
        let before = json!({"kind":"weapon","level":1,"exp":0,"ascension":0,"refinement":1});
        let after = json!({"kind":"weapon","level":1,"exp":0,"ascension":0,"refinement":2});
        assert_eq!(
            item_action(&json!({"previous":before,"item":after})),
            "Изменено: Пробуждение"
        );
        assert!(item_description(&before).ends_with("возв. 0 · R1"));
        assert!(item_description(&after).ends_with("возв. 0 · R2"));
        let ascended = json!({"kind":"weapon","level":20,"exp":0,"ascension":1,"refinement":2});
        assert!(item_description(&ascended).contains("возв. 1 · R2"));
    }
    #[test]
    fn exp_only_update_and_missing_legacy_exp_are_visible() {
        let old = json!({"kind":"artifact","level":19,"exp":4500});
        let new = json!({"kind":"artifact","level":19,"exp":18360});
        assert_eq!(
            item_action(&json!({"previous":old,"item":new})),
            "Изменено: Опыт"
        );
        assert_eq!(item_description(&new), "ур. 19 · опыт 18360");
        assert_eq!(
            item_description(&json!({"kind":"weapon","level":3})),
            "ур. 3 · опыт —"
        );
    }
    #[test]
    fn timestamp_is_preserved_and_legacy_timestamp_is_optional() {
        let stamp = "2026-11-12T03:41:55.469+03:00";
        assert_eq!(
            event_timestamp(&json!({"timestamp":stamp,"observed_unix_ms":"0"})).as_deref(),
            Some(stamp)
        );
        assert!(event_timestamp(&json!({"observed_unix_ms":"1794444115469"})).is_some());
        assert!(event_timestamp(&json!({})).is_none());
    }
}
