export const collections = {
  material_stacks: "Материалы",
  weapons: "Оружие",
  artifacts: "Артефакты",
  furniture: "Декор",
  cosmetics: "Косметика",
  unknown_items: "Другие предметы",
  characters: "Персонажи",
  traveler_depots: "Стихии Путешественника",
  wallet: "Валюты",
  teams: "Отряды",
  player_profile: "Профиль игрока",
  adventure: "Приключения",
  achievements: "Достижения",
};
export const regions = {
  os_euro: "Европа",
  os_asia: "Азия",
  os_usa: "Америка",
  os_cht: "Тайвань / Гонконг / Макао",
};
export const freshness = {
  synchronized: "Синхронизировано",
  saved: "Сохранённые данные",
  stale: "Данные устарели",
  error: "Ошибка источника",
  awaiting_snapshot: "Ожидается снимок",
};
export function newer(a, b) {
  return BigInt(a) > BigInt(b);
}
export function compareEntities(before, after) {
  const changes = [];
  for (const key of new Set([...Object.keys(before), ...Object.keys(after)])) {
    const a = before[key],
      b = after[key];
    if (JSON.stringify(a) === JSON.stringify(b)) continue;
    changes.push({
      key,
      before: a,
      after: b,
      kind: a === undefined ? "added" : b === undefined ? "removed" : "changed",
      delta:
        typeof a?.count === "number" && typeof b?.count === "number"
          ? b.count - a.count
          : null,
    });
  }
  return changes;
}
export function aggregate(rows) {
  const out = new Map();
  for (const row of rows) {
    const id = String(row.values.item_id);
    const old = out.get(id);
    if (old) {
      old.values.count += Number(row.values.count ?? 0);
      old.stacks++;
      old.guids.push(row.entity_key);
      if (row.confirmation !== "observed")
        old.confirmation = "historical_unconfirmed";
    } else
      out.set(id, {
        ...row,
        entity_key: id,
        stacks: 1,
        guids: [row.entity_key],
        values: { ...row.values, count: Number(row.values.count ?? 0) },
      });
  }
  return [...out.values()];
}
export function characterLevel(v) {
  return Number(v.level ?? v.props?.["4001"]?.ival ?? 0);
}
export function characterConstellation(v) {
  return v.constellation ?? v.talent_ids?.length ?? 0;
}
export function relevant(change, profile) {
  return (
    (change.profile_id === profile &&
      change.topics.some((t) => t !== "capture_health")) ||
    change.topics.includes("active_profile") ||
    change.topics.includes("freshness")
  );
}
export function describeOperation(o, name) {
  const old = o.expected ?? {},
    next = o.values ?? {},
    v = next.item_id || next.avatar_id ? next : old;
  const title = v.item_id || v.avatar_id ? name(v) : o.entity_key;
  if (old.fight_props && next.fight_props) {
    const changed = Object.keys(next.fight_props).filter(
      (id) => old.fight_props[id] !== next.fight_props[id],
    );
    if (changed.length)
      return `${title} · боевые характеристики · изменено: ${changed.length}`;
  }
  if (o.action === "delete")
    return `${title} · удалён${old.count !== undefined ? " (было " + old.count + ")" : ""}`;
  if (Array.isArray(old.equip_guids) && Array.isArray(next.equip_guids)) {
    const before = new Set(old.equip_guids),
      after = new Set(next.equip_guids);
    const removed = [...before].filter((id) => !after.has(id)).length;
    const added = [...after].filter((id) => !before.has(id)).length;
    if (removed || added)
      return `${title} · смена экипировки · снято: ${removed} · надето: ${added}`;
  }
  if (next.count !== undefined)
    return `${title} · ${old.count ?? "?"} → ${next.count}`;
  if (next.level !== undefined)
    return `${title} · ур. ${old.level ?? "?"} → ${next.level}`;
  return title;
}
