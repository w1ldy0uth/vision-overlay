import test from "node:test";
import assert from "node:assert/strict";
import {
  adventureExperience,
  completedAchievements,
  showcaseSlots,
} from "../src/player-profile.mjs";
test("adventure EXP uses full threshold, preserves zero and does not invent missing EXP", () => {
  const ref = { adventureExp: { 12: 2600 } };
  assert.equal(adventureExperience({ rank: 12, exp: 400 }, ref), "400/2600");
  assert.equal(adventureExperience({ rank: 12, exp: 0 }, ref), "0/2600");
  assert.equal(adventureExperience({ rank: 12, exp: null }, ref), null);
  assert.equal(adventureExperience({ rank: 12, exp: 400 }, {}), null);
  assert.equal(adventureExperience({ rank: 60, exp: 0 }, ref), "340125/340125");
});
test("showcase preserves twelve ordered slots and links only owned characters", () => {
  const trial = {
    entity_key: "trial",
    values: { avatar_id: 1, trial_avatar_info: {} },
  };
  const owned = { entity_key: "owned", values: { avatar_id: 1 } };
  const slots = showcaseSlots(
    [{ avatar_id: 1 }, { avatar_id: 2 }],
    [trial, owned],
  );
  assert.equal(slots.length, 12);
  assert.equal(slots[0].character, owned);
  assert.equal(slots[1].character, undefined);
  assert.equal(slots[2], null);
});
test("completed achievements include live completions and exclude catalog-only records", () => {
  const data = {
    collections: {
      achievements: [
        { entity_key: "a", values: { status_code: 2 } },
        { entity_key: "b", values: { status_code: 3 } },
        { entity_key: "c", values: { status_code: 1 } },
      ],
    },
  };
  assert.equal(completedAchievements(data, { achievement_count: 1 }, {}), 2);
  assert.equal(completedAchievements(data, {}, { b: { excluded: true } }), 1);
  assert.equal(completedAchievements({ collections: {} }, {}, {}), null);
});
