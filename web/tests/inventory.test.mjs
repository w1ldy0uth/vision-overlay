import test from "node:test";
import assert from "node:assert/strict";
import {
  classifyItem,
  groupInventory,
  categoryDomain,
  inventoryCategories,
  developerInventoryCategories,
  compareGameOrder,
  itemSubtype,
  subtypeOptions,
} from "../src/inventory.mjs";

const definition = (materialType) => ({
  itemType: "ITEM_MATERIAL",
  materialType,
});
const row = (id, guid, count) => ({
  entity_key: guid,
  confirmation: "observed",
  values: { item_id: id, count },
});

test("catalog order keeps groups, descends rarity, and remains stable across quantity updates", () => {
  const definitions = {
    1: { rank: 100, rarity: 3 },
    2: { rank: 100, rarity: 4 },
    3: { rank: 101, rarity: 5 },
  };
  const items = [
    row(3, "33", 1),
    row(1, "11", 900),
    row(2, "22", 2),
    row(9, "99", 1),
  ];
  const sort = (rows) =>
    [...rows]
      .sort((a, b) => compareGameOrder(a, b, definitions))
      .map((r) => r.values.item_id);
  assert.deepEqual(sort(items), [2, 1, 3, 9]);
  assert.deepEqual(
    sort(items.map((r) => ({ ...r, values: { ...r.values, count: 0 } }))),
    [2, 1, 3, 9],
  );
  const weapons = [
    { ...row(1, "100", 1), source_collection: "material_stacks" },
    {
      ...row(2, "22", 1),
      source_collection: "weapons",
      values: { item_id: 2, level: 20 },
    },
    {
      ...row(2, "11", 1),
      source_collection: "weapons",
      values: { item_id: 2, level: 80 },
    },
  ];
  assert.deepEqual(
    [...weapons]
      .sort((a, b) => compareGameOrder(a, b, definitions))
      .map((r) => r.entity_key),
    ["11", "22", "100"],
  );
});

test("quest items with omitted zero rank precede rank ten without promoting unknown definitions", () => {
  // User's first game row: seven zero-rank quest items, then the realm permit.
  const first = [100584, 100585, 100957, 120127, 129001, 129002, 129003];
  const definitions = Object.fromEntries(
    first.map((id) => [id, { rank: null, rarity: 5 }]),
  );
  definitions[101515] = { rank: 10, rarity: 5 };
  definitions[121234] = { rank: 10, rarity: 5 };
  definitions[100788] = { rank: 10, rarity: 4 };
  const input = [
    100788,
    121234,
    999999,
    ...first.slice().reverse(),
    101515,
  ].map((id) => row(id, String(id), 1));
  const ordered = [...input].sort((a, b) =>
    compareGameOrder(a, b, definitions),
  );
  assert.deepEqual(
    ordered.map((r) => r.values.item_id),
    [...first, 101515, 121234, 100788, 999999],
  );
  definitions[100584].rank = 0;
  assert.deepEqual(
    [...input].sort((a, b) => compareGameOrder(a, b, definitions)),
    ordered,
  );
});

test("subcategories use explicit type labels, weapon type and artifact slot without guessing missing definitions", () => {
  const definitions = {
    1: { typeLabel: "Кухонный ингредиент" },
    2: { weaponType: "WEAPON_BOW" },
    3: { equipType: "EQUIP_NECKLACE" },
  };
  assert.equal(itemSubtype(row(1, "1", 1), definitions), "Кухонный ингредиент");
  assert.equal(itemSubtype(row(2, "2", 1), definitions), "Стрелковое оружие");
  assert.equal(itemSubtype(row(3, "3", 1), definitions), "Перо смерти");
  assert.equal(itemSubtype(row(4, "4", 1), definitions), "Тип не определён");
  assert.deepEqual(
    subtypeOptions([row(1, "1", 1), row(1, "2", 5)], definitions),
    [["Кухонный ингредиент", 2]],
  );
});

test("game tabs classify semantic material types and keep all nine tabs plus fallback", () => {
  assert.equal(inventoryCategories.length, 10);
  for (const [type, category] of [
    ["MATERIAL_AVATAR_MATERIAL", "development"],
    ["MATERIAL_EXP_FRUIT", "development"],
    ["MATERIAL_WEAPON_EXP_STONE", "weapons"],
    ["MATERIAL_NOTICE_ADD_HP", "food"],
    ["MATERIAL_EXCHANGE", "resources"],
    ["MATERIAL_WOOD", "resources"],
    ["MATERIAL_WIDGET", "tools"],
    ["MATERIAL_QUEST", "quest"],
    ["MATERIAL_ARANARA", "quest"],
    ["MATERIAL_DESHRET_MANUAL", "quest"],
    ["MATERIAL_HOLIDAY_MEMORY_BOOK", "quest"],
    ["MATERIAL_PHOTO_DISPLAY_BOOK", "quest"],
    ["MATERIAL_WORKER_NOTEBOOK", "quest"],
    ["MATERIAL_MAGIC_STORY_BOOK", "quest"],
    ["MATERIAL_ODETTE_QUEST_PHOTO_BOOK", "quest"],
    ["MATERIAL_HOLIDAY_RESORT_INVITE", "quest"],
    ["MATERIAL_GREATEFESTIVALV2_INVITE", "quest"],
    ["MATERIAL_NATLAN_RELATION_B", "quest"],
    ["MATERIAL_MOON_NIGHT_CARD", "quest"],
    ["MATERIAL_REMUS_MUSIC_BOX", "quest"],
    ["MATERIAL_VESNA_FASHION", "precious"],
    ["MATERIAL_ELEM_CRYSTAL", "precious"],
    ["MATERIAL_FURNITURE_FORMULA", "precious"],
  ])
    assert.equal(classifyItem("material_stacks", definition(type)), category);
  assert.equal(
    classifyItem("material_stacks", definition("NEW_UNKNOWN_TYPE")),
    "other",
  );
  assert.equal(classifyItem("unknown_items", undefined), "other");
  assert.equal(
    classifyItem("material_stacks", {
      ...definition("MATERIAL_CONSUME"),
      systemType: "MATERIAL_SYS_TYPE_BEYOND",
    }),
    "other",
  );
});

test("storage instances keep their type; identical names/IDs do not merge equipment or sources", () => {
  const collections = {
    material_stacks: [row(1, "11", 2), row(1, "12", 3)],
    weapons: [row(1, "21", 1), row(1, "22", 1)],
    unknown_items: [row(9, "31", 1)],
  };
  const before = JSON.stringify(collections);
  const groups = groupInventory(collections, {
    1: definition("MATERIAL_FOOD"),
  });
  assert.equal(groups.food.length, 1);
  assert.equal(groups.food[0].values.count, 5);
  assert.deepEqual(groups.food[0].guids, ["11", "12"]);
  assert.equal(groups.food[0].source_collection, "material_stacks");
  assert.equal(groups.weapons.length, 2);
  assert.equal(groups.other.length, 1);
  assert.equal(JSON.stringify(collections), before);
  const updated = groupInventory(
    { ...collections, material_stacks: [row(1, "12", 4), row(2, "13", 1)] },
    { 1: definition("MATERIAL_FOOD"), 2: definition("MATERIAL_EXCHANGE") },
  );
  assert.equal(updated.food[0].values.count, 4);
  assert.equal(updated.resources[0].values.count, 1);
  assert.equal(updated.weapons.length, 2);
});

test("a game's mixed category cannot conceal errors or claim full coverage from one collection", () => {
  const domains = [
    {
      collection: "material_stacks",
      freshness: "synchronized",
      scope: "full",
      observed_unix_ms: "200",
    },
    {
      collection: "unknown_items",
      freshness: "error",
      scope: "unverified",
      observed_unix_ms: "100",
    },
  ];
  const status = categoryDomain(
    "other",
    [{ source_collection: "unknown_items" }],
    domains,
  );
  assert.equal(status.freshness, "error");
  assert.equal(status.scope, "unverified");
  assert.equal(status.observed_unix_ms, "100");
  assert.equal(categoryDomain("food", [], domains).freshness, "synchronized");
  assert.equal(categoryDomain("weapons", [], domains), null);
});

test("fireworks and seed-dispensary contents stay stored outside ordinary tabs", () => {
  const collections = {
    material_stacks: [
      row(223001, "100", 1),
      row(120791, "101", 1),
      row(100011, "102", 7),
      row(130002, "103", 17),
    ],
  };
  const groups = groupInventory(collections, {
    223001: definition("MATERIAL_FIREWORKS"),
    120791: definition("MATERIAL_REMUS_MUSIC_BOX"),
    100011: definition("MATERIAL_EXCHANGE"),
    130002: definition("MATERIAL_HOME_SEED"),
  });
  assert.equal(groups.non_inventory.length, 2);
  assert.equal(groups.quest.length, 1);
  assert.equal(groups.resources.length, 1);
  assert.equal(groups.other.length, 0);
  assert.ok(!inventoryCategories.some(([id]) => id === "non_inventory"));
  assert.ok(
    developerInventoryCategories.some(([id]) => id === "non_inventory"),
  );
  assert.equal(Object.values(groups).flat().length, 4);
  assert.equal(collections.material_stacks.length, 4);
});

test("weapon ores share the weapons tab without merging instances; verified Traveler memory is not a bag entry", () => {
  const input = {
    weapons: [row(11401, "100", 1), row(11401, "101", 1)],
    material_stacks: [
      row(104011, "102", 3),
      row(104012, "103", 4),
      row(104013, "104", 5),
      row(915, "105", 1),
      row(104003, "106", 10),
    ],
  };
  const defs = Object.fromEntries(
    [104011, 104012, 104013].map((id) => [
      id,
      definition("MATERIAL_WEAPON_EXP_STONE"),
    ]),
  );
  defs[915] = definition("MATERIAL_TALENT");
  defs[104003] = definition("MATERIAL_EXP_FRUIT");
  const groups = groupInventory(input, defs);
  assert.equal(groups.weapons.length, 5);
  assert.equal(
    groups.weapons.filter((r) => r.source_collection === "weapons").length,
    2,
  );
  assert.equal(groups.development.length, 1);
  assert.equal(groups.development[0].values.item_id, 104003);
  assert.equal(groups.non_inventory.length, 1);
  assert.equal(groups.non_inventory[0].values.item_id, 915);
  assert.equal(Object.values(groups).flat().length, 7);
});
