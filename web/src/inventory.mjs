import { aggregate } from "./model.mjs";

export const inventoryCategories = [
  ["weapons", "Оружие"],
  ["artifacts", "Артефакты"],
  ["development", "Материалы развития персонажа"],
  ["food", "Еда"],
  ["resources", "Ресурсы"],
  ["tools", "Инструменты"],
  ["quest", "Предметы заданий"],
  ["precious", "Драгоценности"],
  ["furniture", "Декор"],
  ["other", "Другое"],
];
export const developerInventoryCategories = [
  ["non_inventory", "Вне игрового инвентаря"],
];

const weaponTypes = {
  WEAPON_SWORD_ONE_HAND: "Одноручное оружие",
  WEAPON_CLAYMORE: "Двуручное оружие",
  WEAPON_POLE: "Древковое оружие",
  WEAPON_BOW: "Стрелковое оружие",
  WEAPON_CATALYST: "Катализаторы",
};
const slots = {
  EQUIP_BRACER: "Цветок жизни",
  EQUIP_NECKLACE: "Перо смерти",
  EQUIP_SHOES: "Пески времени",
  EQUIP_RING: "Кубок пространства",
  EQUIP_DRESS: "Корона разума",
};
export function itemSubtype(row, items) {
  const d = items[String(row.values.item_id)] ?? {};
  return (
    weaponTypes[d.weaponType] ??
    slots[d.equipType] ??
    d.typeLabel ??
    "Тип не определён"
  );
}
export function subtypeOptions(rows, items) {
  const counts = new Map();
  for (const row of rows) {
    const type = itemSubtype(row, items);
    counts.set(type, (counts.get(type) ?? 0) + 1);
  }
  return [...counts].sort(([a], [b]) => a.localeCompare(b, "ru"));
}
export function compareGameOrder(a, b, items) {
  const x = items[String(a.values.item_id)] ?? {},
    y = items[String(b.values.item_id)] ?? {};
  const rarity = (d) => Number(d.rarity ?? 0);
  // Known definitions omit a zero-valued rank in the exported game tables.
  // Only a genuinely missing definition belongs after all known priorities.
  const rank = (d) =>
    Object.hasOwn(d, "rank") ? (d.rank ?? 0) : Number.MAX_SAFE_INTEGER;
  const equipment = (r) =>
    ["weapons", "artifacts"].includes(r.source_collection);
  let order = 0;
  if (equipment(a) || equipment(b)) {
    // Equipment first; enhancement ore remains in the same tab after weapons.
    order =
      Number(equipment(b)) - Number(equipment(a)) ||
      rarity(y) - rarity(x) ||
      Number(b.values.level ?? 0) - Number(a.values.level ?? 0);
    if (
      !order &&
      a.source_collection === "artifacts" &&
      b.source_collection === "artifacts"
    )
      order = Number(y.setId ?? 0) - Number(x.setId ?? 0);
  }
  // rank groups materials; rarity descends inside a group (e.g. talent books).
  return (
    order ||
    rank(x) - rank(y) ||
    rarity(y) - rarity(x) ||
    Number(a.values.item_id ?? 0) - Number(b.values.item_id ?? 0) ||
    String(a.entity_key).localeCompare(String(b.entity_key), "en", {
      numeric: true,
    })
  );
}

// Explicit semantic types, never item-ID ranges, localized names or sorting rank.
const rules = {
  weapons: ["MATERIAL_WEAPON_EXP_STONE"],
  development: [
    "MATERIAL_AVATAR_MATERIAL",
    "MATERIAL_EXP_FRUIT",
    "MATERIAL_RELIQUARY_MATERIAL",
    "MATERIAL_AVATAR_TALENT_MATERIAL",
    "MATERIAL_TALENT",
    "MATERIAL_RARE_GROWTH_MATERIAL",
    "MATERIAL_FIRE_MASTER_AVATAR_TALENT_ITEM",
  ],
  food: ["MATERIAL_FOOD", "MATERIAL_NOTICE_ADD_HP", "MATERIAL_SPICE_FOOD"],
  resources: ["MATERIAL_EXCHANGE", "MATERIAL_WOOD", "MATERIAL_FISH_BAIT"],
  tools: ["MATERIAL_WIDGET", "MATERIAL_FISH_ROD"],
  quest: [
    "MATERIAL_QUEST",
    "MATERIAL_QUEST_EVENT_BOOK",
    "MATERIAL_QUEST_ALBUM",
    "MATERIAL_ARANARA",
    "MATERIAL_DESHRET_MANUAL",
    // Confirmed against the game by the user on 2026-11-14.
    "MATERIAL_HOLIDAY_MEMORY_BOOK",
    "MATERIAL_PHOTO_DISPLAY_BOOK",
    "MATERIAL_WORKER_NOTEBOOK",
    "MATERIAL_MAGIC_STORY_BOOK",
    "MATERIAL_ODETTE_QUEST_PHOTO_BOOK",
    "MATERIAL_HOLIDAY_RESORT_INVITE",
    "MATERIAL_GREATEFESTIVALV2_INVITE",
    "MATERIAL_NATLAN_RELATION_B",
    "MATERIAL_MOON_NIGHT_CARD",
    "MATERIAL_REMUS_MUSIC_BOX",
  ],
  precious: [
    "MATERIAL_CONSUME",
    "MATERIAL_CONSUME_BATCH_USE",
    "MATERIAL_ELEM_CRYSTAL",
    "MATERIAL_CHEST",
    "MATERIAL_SELECTABLE_CHEST",
    "MATERIAL_CHEST_BATCH_USE",
    "MATERIAL_CHEST_BATCH_USE_WITH_GROUP",
    "MATERIAL_FURNITURE_FORMULA",
    "MATERIAL_FURNITURE_SUITE_FORMULA",
    "MATERIAL_BGM",
    "MATERIAL_RENAME_ITEM",
    "MATERIAL_VESNA_FASHION",
  ],
};
const typeCategory = Object.fromEntries(
  Object.entries(rules).flatMap(([category, types]) =>
    types.map((type) => [type, category]),
  ),
);

export function classifyItem(collection, definition, itemId) {
  if (["weapons", "artifacts", "furniture"].includes(collection))
    return collection;
  if (
    !definition ||
    !["ITEM_MATERIAL", "ITEM_VIRTUAL"].includes(definition.itemType)
  )
    return "other";
  // Exclude the other game system from ordinary material rules without hiding it.
  if (
    definition.systemType &&
    definition.systemType !== "MATERIAL_SYS_TYPE_NORMAL"
  )
    return "other";
  // Verified individual exception: Traveler constellation item, not a bag entry.
  if (String(itemId) === "915") return "non_inventory";
  // Confirmed: launcher fireworks and seed-dispensary contents are not bag tabs.
  if (
    ["MATERIAL_FIREWORKS", "MATERIAL_HOME_SEED"].includes(
      definition.materialType,
    )
  )
    return "non_inventory";
  return typeCategory[definition.materialType] ?? "other";
}

export function groupInventory(collections, definitions) {
  const groups = Object.fromEntries(
    [...inventoryCategories, ...developerInventoryCategories].map(([id]) => [
      id,
      [],
    ]),
  );
  for (const collection of [
    "weapons",
    "artifacts",
    "material_stacks",
    "furniture",
    "unknown_items",
    "cosmetics",
  ]) {
    const rows =
      collection === "material_stacks"
        ? aggregate(collections[collection] ?? [])
        : (collections[collection] ?? []);
    for (const row of rows) {
      const category = classifyItem(
        collection,
        definitions[String(row.values.item_id)],
        row.values.item_id,
      );
      groups[category].push({ ...row, source_collection: collection });
    }
  }
  return groups;
}

export function categoryDomain(category, rows, domains) {
  const sources = new Set(rows.map((row) => row.source_collection));
  sources.add(
    ["weapons", "artifacts", "furniture"].includes(category)
      ? category
      : "material_stacks",
  );
  if (category === "other") sources.add("unknown_items");
  const present = domains.filter((domain) => sources.has(domain.collection));
  if (!present.length) return null;
  const rank = {
    synchronized: 0,
    saved: 1,
    awaiting_snapshot: 2,
    stale: 3,
    error: 4,
  };
  const worst = present.reduce((a, b) =>
    (rank[b.freshness] ?? 4) > (rank[a.freshness] ?? 4) ? b : a,
  );
  const times = present.map((d) => d.observed_unix_ms).filter(Boolean);
  return {
    ...worst,
    freshness:
      present.length < sources.size && (rank[worst.freshness] ?? 4) < 2
        ? "awaiting_snapshot"
        : worst.freshness,
    scope:
      present.length === sources.size &&
      present.every((d) => d.scope === "full")
        ? "full"
        : "unverified",
    observed_unix_ms: times.length
      ? times.reduce((a, b) => (BigInt(a) < BigInt(b) ? a : b))
      : undefined,
  };
}
