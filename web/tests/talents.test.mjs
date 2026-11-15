import { test } from "node:test";
import assert from "node:assert/strict";
import { talentLevels } from "../src/talents.mjs";
const catalog = {
  skillMetadata: {
    11: { proudSkillGroupId: 101 },
    12: { proudSkillGroupId: 102 },
    19: { proudSkillGroupId: 109 },
  },
  skillDepots: {
    1: {
      skills: [11, 12],
      energySkill: 19,
      talents: [1, 2, 3, 4, 5, 6],
      constellationBonuses: {
        3: [
          { skillId: 19, proudSkillGroupId: 109, levels: 3, constellation: 3 },
        ],
        5: [
          { skillId: 12, proudSkillGroupId: 102, levels: 3, constellation: 5 },
        ],
      },
    },
  },
};
test("C3 and C5 target explicit skills; base levels are never changed or added twice", () => {
  const p = {
    skill_depot_id: 1,
    skill_levels: { 11: 6, 12: 8, 19: 10 },
    talent_ids: [1, 2, 3, 4, 5],
    extra_skill_levels: { 102: 3, 109: 3 },
  };
  const rows = talentLevels(p, catalog);
  assert.deepEqual(
    rows.map((r) => r.total),
    [6, 11, 13],
  );
  assert.deepEqual(
    rows.map((r) => r.base),
    [6, 8, 10],
  );
  assert.equal(rows[2].constellation, 3);
  assert.equal(rows[1].constellation, 5);
  assert.equal(p.skill_levels[19], 10);
});
test("conditional +1 and combined +4 do not affect persistent displayed levels", () => {
  const rows = talentLevels(
    {
      skill_depot_id: 1,
      skill_levels: { 11: 6, 19: 6 },
      talent_ids: [3],
      extra_skill_levels: { 101: 1, 109: 4 },
    },
    catalog,
  );
  assert.deepEqual(
    rows.map((r) => r.total),
    [6, 9],
  );
  assert(rows.every((r) => r.conditionalIgnored));
});
test("constellation number/count alone and another depot cannot grant a bonus", () => {
  assert.equal(
    talentLevels(
      {
        skill_depot_id: 1,
        skill_levels: { 19: 6 },
        talent_ids: [1, 2, 4],
        extra_skill_levels: { 109: 3 },
      },
      catalog,
    )[0].bonus,
    0,
  );
  assert.equal(
    talentLevels(
      { skill_depot_id: 2, skill_levels: { 19: 6 }, talent_ids: [3] },
      catalog,
    )[0].bonus,
    0,
  );
  assert(
    talentLevels(
      {
        skill_depot_id: 1,
        skill_levels: { 19: 6 },
        talent_ids: [],
        extra_skill_levels: { 109: 3 },
      },
      catalog,
    )[0].unresolved,
  );
});
test("new characters can have C3 on normal attacks; absent metadata falls back to base", () => {
  const c = structuredClone(catalog);
  c.skillDepots[1].constellationBonuses[3][0] = {
    skillId: 11,
    proudSkillGroupId: 101,
    levels: 3,
    constellation: 3,
  };
  assert.equal(
    talentLevels(
      { skill_depot_id: 1, skill_levels: { 11: 4 }, talent_ids: [3] },
      c,
    )[0].total,
    7,
  );
  assert.equal(
    talentLevels(
      { skill_levels: { 11: 4 }, extra_skill_levels: { 101: 3 } },
      {},
    )[0].total,
    4,
  );
});
