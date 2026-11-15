import test from "node:test";
import assert from "node:assert/strict";
import { achievementGroups, achievementMatches } from "../src/achievements.mjs";

const row = (id, status) => ({
  entity_key: String(id),
  values: { status_code: status, current_progress: 2, total_progress: 10 },
});
const catalog = {
  11: {
    chain_id: "11",
    stage: 1,
    category_order: 2,
    goal_id: 1,
    chain_order: 5,
    name: "Same title",
    description: "First target",
  },
  12: {
    chain_id: "11",
    stage: 2,
    category_order: 2,
    goal_id: 1,
    chain_order: 5,
    name: "Same title",
    description: "Second target",
  },
  13: {
    chain_id: "11",
    stage: 3,
    category_order: 2,
    goal_id: 1,
    chain_order: 5,
    name: "Same title",
    description: "Third target",
  },
  21: {
    chain_id: "21",
    stage: 1,
    category_order: 1,
    goal_id: 7,
    chain_order: 9,
    name: "Same title",
  },
  22: {
    chain_id: "22",
    stage: 1,
    category_order: 2,
    goal_id: 1,
    chain_order: 2,
    name: "Same title",
  },
  84517: { excluded: true, name: "Мгновенная карма" },
};

test("linked tiers group without merging equal titles; categories and cards use game order", () => {
  const groups = achievementGroups(
    [row(13, 1), row(21, 3), row(12, 2), row(11, 3), row(84517, 1), row(22, 3)],
    catalog,
  );
  assert.deepEqual(
    groups.map((g) => g.key),
    ["21", "22", "11"],
  );
  assert.deepEqual(
    groups[2].stages.map((s) => s.entity_key),
    ["11", "12", "13"],
  );
});
test("filtering a chain preserves completed tiers as context and searches every tier", () => {
  const [group] = achievementGroups([row(11, 3), row(12, 1)], catalog);
  assert.equal(
    achievementMatches(group, catalog, "unearned", "1", "First target"),
    true,
  );
  assert.equal(
    achievementMatches(group, catalog, "finished", "all", ""),
    false,
  );
  assert.equal(achievementMatches(group, catalog, "all", "7", ""), false);
  assert.equal(group.stages.length, 2);
  assert.equal(group.levels[2].row, undefined); // Missing observation is not zero progress.
});
