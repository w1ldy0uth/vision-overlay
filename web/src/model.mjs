import { tr } from "./i18n.mjs";
export const collections = {
  material_stacks: tr("Материалы"),
  weapons: tr("Оружие"),
  artifacts: tr("Артефакты"),
  furniture: tr("Декор"),
  cosmetics: tr("Косметика"),
  unknown_items: tr("Другие предметы"),
  characters: tr("Персонажи"),
  traveler_depots: tr("Стихии Путешественника"),
  wallet: tr("Валюты"),
  teams: tr("Отряды"),
  player_profile: tr("Профиль игрока"),
  adventure: tr("Приключения"),
  achievements: tr("Достижения"),
};
export const regions = {
  os_euro: tr("Европа"),
  os_asia: tr("Азия"),
  os_usa: tr("Америка"),
  os_cht: tr("Тайвань / Гонконг / Макао"),
};
export const freshness = {
  synchronized: tr("Синхронизировано"),
  saved: tr("Сохранённые данные"),
  stale: tr("Данные устарели"),
  error: tr("Ошибка источника"),
  awaiting_snapshot: tr("Ожидается снимок"),
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
      return tr("{0} · боевые характеристики · изменено: {1}", title, changed.length);
  }
  if (o.action === "delete")
    return tr("{0} · удалён{1}", title, old.count !== undefined ? tr(" (было ") + old.count + ")" : "");
  if (Array.isArray(old.equip_guids) && Array.isArray(next.equip_guids)) {
    const before = new Set(old.equip_guids),
      after = new Set(next.equip_guids);
    const removed = [...before].filter((id) => !after.has(id)).length;
    const added = [...after].filter((id) => !before.has(id)).length;
    if (removed || added)
      return tr("{0} · смена экипировки · снято: {1} · надето: {2}", title, removed, added);
  }
  if (next.count !== undefined)
    return `${title} · ${old.count ?? "?"} → ${next.count}`;
  if (next.level !== undefined)
    return tr("{0} · ур. {1} → {2}", title, old.level ?? "?", next.level);
  return title;
}
