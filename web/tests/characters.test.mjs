import test from "node:test";
import assert from "node:assert/strict";
import {
  characterGallery,
  characterElement,
  characterPriority,
  savedFavorites,
  matchesCharacterElements,
} from "../src/characters.mjs";
const row = (id, extra = {}) => ({
  entity_key: String(id),
  values: { avatar_id: id, ...extra },
});
const catalog = {
  avatarMetadata: Object.fromEntries(
    [10000005, 10000007, 10000010, 10000011, 10000134, 10000135, 10000901].map(
      (id) => [id, { useType: "AVATAR_FORMAL", skillDepotId: 1 }],
    ),
  ),
  avatarNames: { 10000010: "A", 10000011: "B", 10000901: "Trial" },
  avatarIcons: { 10000010: "A", 10000011: "B", 10000901: "Trial" },
  skillDepots: { 1: { energySkill: 10 }, 2: { energySkill: 20 } },
  skillMetadata: { 10: { element: "Fire" }, 20: { element: "Ice" } },
};
test("gallery appends missing characters without creating inventory facts or duplicate travelers", () => {
  const owned = row(10000010),
    gallery = characterGallery(
      [
        owned,
        row(10000005),
        row(10000134),
        row(10000011, { trial_avatar_info: {} }),
      ],
      catalog,
    );
  assert.equal(gallery[0], owned);
  assert.equal(gallery.length, 3);
  assert.equal(gallery[2].values.avatar_id, 10000011);
  assert.equal(gallery[2].unowned, true);
  assert.equal(gallery[2].values.level, undefined);
});
test("favorites lead, remaining owned characters precede unowned, and active depot determines element", () => {
  const owned = row(10000010),
    missing = { ...row(10000011), unowned: true };
  assert.ok(characterPriority(owned, missing, []) < 0);
  assert.ok(characterPriority(missing, owned, [10000011]) > 0);
  assert.equal(characterElement(owned.values, catalog), "Fire");
  assert.equal(
    characterElement({ ...owned.values, skill_depot_id: 2 }, catalog),
    "Ice",
  );
  assert.equal(characterElement({ avatar_id: 123 }, catalog), null);
});
test("favorite preferences retain independent profile lists and tolerate invalid saved data", () => {
  assert.deepEqual(savedFavorites('{"a":[1,1,2,"3"],"b":[4],"c":null}'), {
    a: [1, 2],
    b: [4],
  });
  assert.deepEqual(savedFavorites("invalid"), {});
  assert.deepEqual(savedFavorites("[]"), {});
});

test('multi-element filter uses OR, supports active traveler element, and clears to all',()=>{
 const fire=row(10000010),ice=row(10000011,{skill_depot_id:2});
 assert.equal(matchesCharacterElements(fire,catalog,['Fire','Ice']),true);
 assert.equal(matchesCharacterElements(ice,catalog,['Fire','Ice']),true);
 assert.equal(matchesCharacterElements(fire,catalog,['Ice']),false);
 assert.equal(matchesCharacterElements(row(999),catalog,[]),true);
});
