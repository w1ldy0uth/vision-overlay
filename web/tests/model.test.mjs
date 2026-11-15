import { test } from "node:test";
import assert from "node:assert/strict";
import {
  aggregate,
  compareEntities,
  newer,
  relevant,
  describeOperation,
} from "../src/model.mjs";
test("fight property history recognizes a changed total and an explicit zero", () => {
  assert.match(
    describeOperation(
      {
        action: "set",
        expected: { avatar_id: 1, fight_props: { 2001: 464, 40: 0.2 } },
        values: { avatar_id: 1, fight_props: { 2001: 787, 40: 0 } },
      },
      () => "Валерий",
    ),
    /Валерий · боевые характеристики · изменено: 2/,
  );
});
test("equipment history distinguishes removal and replacement without treating order as a change", () => {
  const operation = (before, after) => ({
    action: "set",
    expected: { avatar_id: 1, equip_guids: before },
    values: { avatar_id: 1, equip_guids: after },
  });
  assert.equal(
    describeOperation(operation(["18446744073709551615"], []), () => "Скирк"),
    "Скирк · смена экипировки · снято: 1 · надето: 0",
  );
  assert.equal(
    describeOperation(operation(["1", "2"], ["2", "3"]), () => "Скирк"),
    "Скирк · смена экипировки · снято: 1 · надето: 1",
  );
  assert.equal(
    describeOperation(operation(["1", "2"], ["2", "1"]), () => "Скирк"),
    "Скирк",
  );
});
test("revision strings retain precision above JS integer limit", () => {
  assert(newer("9007199254740993", "9007199254740992"));
  assert(!newer("3", "3"));
});
test("multiple stacks aggregate without mutating source or concealing unconfirmed values", () => {
  const rows = [
    {
      entity_key: "18446744073709551615",
      confirmation: "observed",
      values: { item_id: 1, count: 3 },
    },
    {
      entity_key: "2",
      confirmation: "historical_unconfirmed",
      values: { item_id: 1, count: 4 },
    },
  ];
  const a = aggregate(rows);
  assert.equal(a[0].values.count, 7);
  assert.equal(a[0].confirmation, "historical_unconfirmed");
  assert.equal(rows[0].values.count, 3);
});
test("comparison represents deletion, creation and a changed artifact field", () => {
  const d = compareEntities(
    { a: { count: 1 }, b: { level: 4 } },
    { b: { level: 8 }, c: { count: 1 } },
  );
  assert.deepEqual(
    d.map((x) => x.kind),
    ["removed", "changed", "added"],
  );
});
test("heartbeat and another profile cannot refresh selected profile facts", () => {
  assert(!relevant({ profile_id: "b", topics: ["material_stacks"] }, "a"));
  assert(!relevant({ profile_id: "a", topics: ["capture_health"] }, "a"));
  assert(relevant({ profile_id: "a", topics: ["material_stacks"] }, "a"));
  assert(relevant({ profile_id: null, topics: ["active_profile"] }, "a"));
});
test("deletion keeps item identity and previous quantity from expected", () => {
  assert.equal(
    describeOperation(
      {
        action: "delete",
        entity_key: "18446744073709551615",
        values: {},
        expected: { item_id: 121374, count: 1 },
      },
      () => "Напиток",
    ),
    "Напиток · удалён (было 1)",
  );
});
