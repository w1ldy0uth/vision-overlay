use crate::event_format::{
    achievement_action, achievement_display, adventure_action, adventure_display, item_action,
    item_description,
};
use anyhow::{Context, Result};
use serde_json::Value;
use std::{collections::HashMap, path::Path};

#[derive(Default)]
pub struct Labels {
    names: HashMap<String, String>,
    skills: HashMap<String, String>,
    achievements: HashMap<String, String>,
}
impl Labels {
    pub fn load(items: &Path, avatars: &Path, achievements: &Path) -> Result<Self> {
        let mut labels = Self::default();
        for (path, fields) in [
            (items, &["names", "item_names"][..]),
            (avatars, &["names", "skill_names"][..]),
        ] {
            if !path.exists() {
                continue;
            }
            let catalog: Value = serde_json::from_slice(
                &std::fs::read(path).context("Cannot read display catalog")?,
            )?;
            for field in fields {
                if let Some(names) = catalog[*field].as_object() {
                    for (id, name) in names {
                        if let Some(name) = name.as_str() {
                            let target = if *field == "skill_names" {
                                &mut labels.skills
                            } else {
                                &mut labels.names
                            };
                            target.insert(
                                id.clone(),
                                name.chars()
                                    .map(|c| if c.is_control() { ' ' } else { c })
                                    .collect(),
                            );
                        }
                    }
                }
            }
        }
        if achievements.exists() {
            let catalog: Value = serde_json::from_slice(&std::fs::read(achievements)?)?;
            for (id, row) in catalog["records"]
                .as_object()
                .context("Missing achievement catalog records")?
            {
                if let Some(name) = row["name"].as_str() {
                    labels.achievements.insert(
                        id.clone(),
                        name.chars()
                            .map(|c| if c.is_control() { ' ' } else { c })
                            .collect(),
                    );
                }
            }
        }
        labels.names.insert("201".into(), "Примогемы".into());
        labels.names.insert("202".into(), "Мора".into());
        Ok(labels)
    }
    fn name(&self, id: &Value) -> String {
        self.names
            .get(&id.to_string())
            .cloned()
            .unwrap_or_else(|| format!("Неизвестный предмет/персонаж (ID {id})"))
    }
    pub fn format(&self, kind: &str, data: &Value) -> String {
        match kind {
            "wish_completed" => format!(
                "Молитва · баннер {}: {}",
                data["gacha_type"].as_str().unwrap_or("?"),
                data["results"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|r| self.name(&r["item_id"]))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),

            "material_changed" => {
                let c = &data["change"];
                format!(
                    "{}: {} → {} ({:+})",
                    self.name(&c["item_id"]),
                    c["previous_count"],
                    c["count"],
                    c["delta"].as_i64().unwrap_or(0)
                )
            }
            "item_changed" => {
                let c = &data["change"];
                format!(
                    "{} · {}: {} → {}",
                    self.name(&c["item_id"]),
                    item_action(c),
                    item_description(&c["previous"]),
                    item_description(&c["item"])
                )
            }
            "currency_snapshot" => format!(
                "{}: {} (исходный баланс)",
                self.name(&data["item_id"]),
                data["count"]
            ),
            "currency_changed" => format!(
                "{}: {} → {} ({:+})",
                self.name(&data["item_id"]),
                data["previous_count"],
                data["count"],
                data["delta"].as_i64().unwrap_or(0)
            ),
            "achievement_changed" => format!(
                "{} · {}: {} → {}",
                self.achievements
                    .get(&data["achievement_id"].to_string())
                    .cloned()
                    .unwrap_or_else(|| format!("Достижение ID {}", data["achievement_id"])),
                achievement_action(data),
                achievement_display(&data["previous"]),
                achievement_display(&data["current"])
            ),
            "adventure_snapshot" => format!(
                "{} (исходное состояние)",
                adventure_display(&data["current"])
            ),
            "adventure_progress_changed" => format!(
                "{}: {} → {}",
                adventure_action(data),
                adventure_display(&data["previous"]),
                adventure_display(&data["current"])
            ),
            "constellation_depot_changed" => format!(
                "{} · Созвездие другого элемента активировано (полное состояние элемента требует снимка)",
                self.name(&data["avatar_id"])
            ),
            "constellation_changed" => format!(
                "{} · Созвездие активировано: C{} → C{}",
                self.name(&data["avatar_id"]),
                data["previous_count"],
                data["count"]
            ),
            "traveler_element_changed" => format!(
                "{} · Набор стихии обновлён · C{}",
                self.name(&data["avatar_id"]),
                data["current"]["talent_ids"].as_array().map_or(0, Vec::len)
            ),
            "fight_properties_changed" => format!(
                "{} · Боевые характеристики: {}",
                self.name(&data["avatar_id"]),
                data["current"]
                    .as_object()
                    .map(|m| m
                        .keys()
                        .map(|k| match k.as_str() {
                            "2000" => "Макс. HP",
                            "2001" => "Атака",
                            "2002" => "Защита",
                            "20" => "Крит. шанс",
                            "22" => "Крит. урон",
                            "23" => "Восст. энергии",
                            "28" => "Мастерство стихий",
                            _ => "Другие",
                        })
                        .collect::<std::collections::BTreeSet<_>>()
                        .into_iter()
                        .collect::<Vec<_>>()
                        .join(", "))
                    .unwrap_or_default()
            ),
            "equipment_changed" => format!(
                "{} · {} · {}",
                self.name(&data["avatar_id"]),
                if data["item_guid"].is_null() {
                    "Снятие экипировки"
                } else {
                    "Смена экипировки"
                },
                if data["item_guid"].is_null() {
                    "Пустой слот".into()
                } else {
                    self.name(&data["item_id"])
                }
            ),
            "character_added" => format!(
                "{} · Получен персонаж · ур. {}",
                self.name(&data["avatar_id"]),
                data["profile"]["props"]["4001"]["ival"]
                    .as_str()
                    .unwrap_or("—")
            ),
            "talent_changed" => format!(
                "{} · талант «{}»: {} → {}",
                self.name(&data["avatar_id"]),
                self.skills
                    .get(&data["skill_id"].to_string())
                    .cloned()
                    .unwrap_or_else(|| format!("Талант ID {}", data["skill_id"])),
                data["previous_level"],
                data["level"]
            ),
            "friendship_changed" | "character_progress_changed" => format!(
                "{} · {}: ур. {} / {} опыта → ур. {} / {} опыта{}",
                self.name(&data["avatar_id"]),
                if kind == "friendship_changed" {
                    "дружба"
                } else {
                    "развитие персонажа"
                },
                data["previous"]["level"],
                data["previous"]["exp"],
                data["current"]["level"],
                data["current"]["exp"],
                data["exp_delta"]
                    .as_i64()
                    .map(|n| format!(" ({n:+})"))
                    .unwrap_or_default()
            ),
            "team_changed" => format!(
                "Отряды: {}",
                data["changes"]
                    .as_array()
                    .map(|rows| rows
                        .iter()
                        .map(|row| format!(
                            "{} · {}",
                            row["team"]["team_id"],
                            row["team"]["name"].as_str().unwrap_or("Без названия")
                        ))
                        .collect::<Vec<_>>()
                        .join(", "))
                    .unwrap_or_default()
            ),
            "player_profile_changed" => format!(
                "Профиль игрока: {}",
                data["profile"]["nickname"]
                    .as_str()
                    .unwrap_or("Обновлены данные")
            ),
            _ => kind.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn achievement_names_do_not_collide_with_items_and_completion_is_visible() {
        let mut labels = Labels::default();
        labels.names.insert("82040".into(), "Другой предмет".into());
        labels
            .achievements
            .insert("82040".into(), "Приятного аппетита".into());
        let text=labels.format("achievement_changed",&json!({"achievement_id":82040,"action":"completed","previous":{"status_code":1,"current_progress":3,"total_progress":4},"current":{"status_code":2,"current_progress":null,"total_progress":4}}));
        assert!(text.contains("Приятного аппетита · Достижение выполнено"));
        assert!(text.contains("4 / 4"));
        assert!(!text.contains("Другой предмет"));
    }
    #[test]
    fn currencies_and_talents_use_their_own_names() {
        let mut labels = Labels::default();
        labels.names.insert("201".into(), "Примогемы".into());
        labels.names.insert("10000071".into(), "Сайно".into());
        labels.names.insert("10715".into(), "Другой предмет".into());
        labels
            .skills
            .insert("10715".into(), "Проворство шакала".into());
        assert_eq!(
            labels.format("currency_snapshot", &json!({"item_id":201,"count":99})),
            "Примогемы: 99 (исходный баланс)"
        );
        assert_eq!(
            labels.format(
                "currency_changed",
                &json!({"item_id":201,"previous_count":99,"count":109,"delta":10})
            ),
            "Примогемы: 99 → 109 (+10)"
        );
        assert_eq!(
            labels.format(
                "talent_changed",
                &json!({"avatar_id":10000071,"skill_id":10715,"previous_level":5,"level":6})
            ),
            "Сайно · талант «Проворство шакала»: 5 → 6"
        );
    }
    #[test]
    fn names_replace_ids_without_hiding_unknown_items() {
        let mut labels = Labels::default();
        labels.names.insert("15402".into(), "Бесструнный".into());
        let data = json!({"change":{"item_id":15402,"previous":{"kind":"weapon","level":1,"exp":0},"item":{"kind":"weapon","level":3,"exp":175}}});
        let line = labels.format("item_changed", &data);
        assert!(line.contains("Бесструнный"));
        assert!(!line.contains("15402"));
        assert!(line.contains("ур. 1 · опыт 0 → ур. 3 · опыт 175"));
        assert!(labels.name(&json!(999)).contains("ID 999"));
    }
}
