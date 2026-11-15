import { test } from "node:test";
import assert from "node:assert/strict";
import { travelerSets, elementName } from "../src/traveler.mjs";
const c = {
  avatarMetadata: { 10000005: { skillDepotIds: [501, 504, 505] } },
  skillDepots: {
    501: { skills: [] },
    504: {
      skills: [1],
      energySkill: 2,
      talents: [1, 2, 3, 4, 5, 6],
      constellationBonuses: {
        3: [{ skillId: 1, proudSkillGroupId: 1, levels: 3, constellation: 3 }],
      },
    },
    505: { skills: [3], energySkill: 4 },
  },
  skillMetadata: {
    1: { proudSkillGroupId: 1 },
    2: { element: "Wind" },
    3: { proudSkillGroupId: 3 },
    4: { element: "Ice" },
  },
};
test("independent observed sets keep the active state, saved data and unknown distinct", () => {
  const v = {
    avatar_id: 10000005,
    guid: "42",
    skill_depot_id: 505,
    skill_levels: { 3: 6 },
    talent_ids: [],
  };
  const d = {
    collections: {
      traveler_depots: [
        {
          confirmation: "not_observed",
          values: {
            avatar_id: 10000005,
            guid: "42",
            skill_depot_id: 504,
            skill_levels: { 1: 4 },
            talent_ids: [1, 2, 3],
          },
        },
      ],
    },
  };
  const sets = travelerSets(v, c, d);
  assert.equal(sets.length, 2);
  assert.equal(sets[0].name, "Анемо");
  assert.equal(sets[0].saved, true);
  assert.equal(sets[0].talents[0].total, 7);
  assert.equal(sets[1].active, true);
  assert.equal(sets[1].talents[0].base, 6);
  assert.equal(travelerSets(v, c, { collections: {} })[0].known, false);
  assert.equal(travelerSets({ ...v, guid: "43" }, c, d)[0].known, false);
  assert.equal(elementName(505, c), "Крио");
  assert.deepEqual(travelerSets({ avatar_id: 10000134 }, c, d), []);
});
