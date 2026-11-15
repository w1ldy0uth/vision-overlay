import { test } from "node:test";
import assert from "node:assert/strict";
import { progressionReference } from "../src/progression.mjs";

const catalog = {
  progression: {
    avatarPaths: { 5: 12, 7: 12 },
    weaponPaths: { 11101: 11101 },
    avatarPromotions: { 12: {
      0: { requiredLevel: 0, maxLevel: 20 },
      1: { requiredLevel: 20, maxLevel: 40, mora: 20000, materials: [{ id: 1, count: 1 }] },
      2: { requiredLevel: 40, maxLevel: 50, mora: 40000, materials: [{ id: 2, count: 3 }] },
    } },
    weaponPromotions: { 11101: {
      0: { requiredLevel: 0, maxLevel: 20 },
      1: { requiredLevel: 20, maxLevel: 40 },
      2: { requiredLevel: 40, maxLevel: 50 },
      3: { requiredLevel: 50, maxLevel: 60 },
      4: { requiredLevel: 60, maxLevel: 70 },
    } },
    talentLevels: {
      501: { 2: { requiredAscension: 2, mora: 12500, materials: [{ id: 301, count: 3 }] } },
      701: { 2: { requiredAscension: 2, mora: 12500, materials: [{ id: 302, count: 3 }] } },
    },
  },
  skillDepots: { 501: { skills: [11], energySkill: 0 }, 507: { skills: [12], energySkill: 0 } },
  skillMetadata: { 11: { proudSkillGroupId: 501 }, 12: { proudSkillGroupId: 701 } },
};
test("Traveler cost schemes use active depot and preserve the shared ascension path", () => {
  const wind = progressionReference({ avatar_id: 5, skill_depot_id: 501 }, catalog);
  const other = progressionReference({ avatar_id: 5, skill_depot_id: 507 }, catalog);
  assert.deepEqual(wind.promotions, other.promotions);
  assert.equal(wind.promotions[0].stage, 1);
  assert.equal(wind.talents[0].levels[0].materials[0].id, 301);
  assert.equal(other.talents[0].levels[0].materials[0].id, 302);
  assert.equal(wind.talents[0].levels[0].requiredLevel, 40);
  assert.equal(wind.talents[0].levels[0].level, 2);
});
test("Short weapon paths end at source limit; missing catalog/depot is never guessed", () => {
  const weapon = progressionReference({ item_id: 11101 }, catalog);
  assert.equal(weapon.promotions.at(-1).maxLevel, 70);
  assert.equal(weapon.promotions.length, 4);
  assert.deepEqual(weapon.talents, []);
  assert.equal(progressionReference({ avatar_id: 5 }, {}), null);
  assert.equal(progressionReference({ item_id: 999 }, catalog), null);
  assert.deepEqual(progressionReference({ avatar_id: 5, skill_depot_id: 999 }, catalog).talents, []);
});
