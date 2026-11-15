import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  expMaterials,
  levelPreview,
  nextUpgrades,
  observedStock,
} from "../src/upgrades.mjs";
// Reduced exact-source fixture, revision a311cd77d7dafbd2bedc7d3cac4e986c6125b586.
const catalog = JSON.parse(
  readFileSync(new URL("./fixtures/leveling.json", import.meta.url)),
);
function data(stock = {}, rank = 60) {
  return {
    collections: {
      material_stacks: Object.entries(stock).map(([id, count]) => ({
        confirmation: "observed",
        values: { item_id: Number(id), count },
      })),
      wallet: [
        {
          confirmation: "observed",
          values: { item_id: 202, count: "10000000" },
        },
      ],
      adventure: [{ values: { rank } }],
    },
    domains: ["material_stacks", "wallet", "characters", "weapons"].map(
      (collection) => ({ collection, freshness: "synchronized" }),
    ),
  };
}
const char = (level, exp, ascension) => ({
  avatar_id: 10000137,
  skill_depot_id: 13701,
  props: {
    4001: { ival: String(level) },
    1001: { ival: String(exp) },
    1002: { ival: String(ascension) },
  },
  skill_levels: {},
});

test("real game previews: Valery 4425/18000 and Forest Regalia 0/10500", () => {
  const l = catalog.leveling;
  assert.equal(l.avatarExp[22], 18000);
  assert.equal(l.weaponExp[4][20], 10500);
  const valery = levelPreview(
    22,
    4425,
    40,
    l.avatarExp,
    [{ id: 104003, count: 1 }],
    l.avatarMaterials,
    l.avatarMoraPerExp,
  );
  assert.equal(valery.mora, 4000);
  assert.equal(valery.resultLevel, 23);
  assert.equal(valery.resultExp, 6425);
  const weapon = levelPreview(
    20,
    0,
    40,
    l.weaponExp[4],
    [{ id: 104013, count: 1 }],
    l.weaponMaterials,
    l.weaponMoraPerExp[4],
    l.weaponReturnMaterials,
  );
  assert.equal(weapon.mora, 1000);
  assert.equal(weapon.resultLevel, 20);
  assert.equal(weapon.resultExp, 10000);
});
test("real Dull Blade cap preview charges all EXP and returns two fine plus four normal ores", () => {
  const l = catalog.leveling;
  const s = levelPreview(
    1,
    0,
    20,
    l.weaponExp[1],
    [{ id: 104013, count: 3 }],
    l.weaponMaterials,
    l.weaponMoraPerExp[1],
    l.weaponReturnMaterials,
  );
  assert.equal(s.resultLevel, 20);
  assert.equal(s.mora, 3000);
  assert.equal(s.resultExp, 0);
  assert.deepEqual(s.returnedMaterials, [
    { id: 104012, count: 2 },
    { id: 104011, count: 4 },
  ]);
  assert.equal(s.lostExp, 75);
});
test("additional game previews match Prune, Recurve Bow and Dragon's Bane", () => {
  const l = catalog.leveling;
  const prune = levelPreview(
    30,
    27075,
    40,
    l.avatarExp,
    [{ id: 104003, count: 1 }],
    l.avatarMaterials,
    l.avatarMoraPerExp,
  );
  assert.equal(l.avatarExp[30], 29100);
  assert.equal(prune.resultLevel, 31);
  assert.equal(prune.mora, 4000);
  for (const [rarity, threshold, level] of [
    [3, 275, 10],
    [4, 400, 8],
  ]) {
    const quote = levelPreview(
      1,
      0,
      20,
      l.weaponExp[rarity],
      [{ id: 104013, count: 1 }],
      l.weaponMaterials,
      l.weaponMoraPerExp[rarity],
      l.weaponReturnMaterials,
    );
    assert.equal(l.weaponExp[rarity][1], threshold);
    assert.equal(quote.resultLevel, level);
    assert.equal(quote.mora, 1000);
  }
});
test("bounded material selection minimizes EXP then item count and does not consume unknown stocks", () => {
  const stock = { 104001: 3, 104002: 14, 104003: 24 };
  const p = expMaterials(
    13575,
    catalog.leveling.avatarMaterials,
    (id) => stock[id] ?? null,
  );
  assert.equal(p.suppliedExp, 15000);
  assert.deepEqual(p.materials, [{ id: 104002, count: 3 }]);
  assert.equal(p.fromStock, true);
  const alternative = expMaterials(
    500,
    { 1: 100, 2: 500, 3: 1000 },
    (id) => ({ 1: 1, 2: 0, 3: 1 })[id],
  );
  assert.equal(alternative.suppliedExp, 1000);
  assert.deepEqual(alternative.materials, [{ id: 3, count: 1 }]);
  const fallback = expMaterials(500, { 1: 100, 2: 500 }, () => null);
  assert.equal(fallback.fromStock, false);
});
test("selection agrees with exhaustive finite inventory search", () => {
  const definitions = { 1: 100, 2: 500, 3: 2000 };
  for (let a = 0; a <= 3; a++)
    for (let b = 0; b <= 3; b++)
      for (let c = 0; c <= 2; c++)
        for (const required of [50, 500, 650, 1600, 2300, 4700]) {
          const stock = { 1: a, 2: b, 3: c };
          const brute = [];
          for (let x = 0; x <= a; x++)
            for (let y = 0; y <= b; y++)
              for (let z = 0; z <= c; z++) {
                const exp = x * 100 + y * 500 + z * 2000;
                if (exp >= required) brute.push([exp, x + y + z]);
              }
          brute.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
          const p = expMaterials(required, definitions, (id) => stock[id]);
          if (brute.length) {
            assert.equal(p.fromStock, true);
            assert.deepEqual(
              [p.suppliedExp, p.materials.reduce((n, m) => n + m.count, 0)],
              brute[0],
            );
          } else assert.equal(p.fromStock, false);
        }
});
test("next action uses remaining EXP, stops for ascension/AR and respects short rarity paths", () => {
  const d = data({ 104001: 3, 104002: 14, 104003: 24 });
  const q = nextUpgrades(char(22, 4425, 1), catalog, d).progress;
  assert.equal(q.neededExp, 13575);
  assert.equal(q.mora, 3000);
  assert.equal(q.resultLevel, 23);
  assert.equal(q.resultExp, 1425);
  assert.equal(q.status, "ready");
  const locked = nextUpgrades(
    { item_id: 12417, level: 20, exp: 0, ascension: 0 },
    catalog,
    data({}, 14),
  ).progress;
  assert.equal(locked.kind, "ascension");
  assert.equal(locked.status, "locked");
  assert.match(locked.blockers[0], /15/);
  assert.equal(
    nextUpgrades(
      { item_id: 11101, level: 70, exp: 0, ascension: 4 },
      catalog,
      d,
    ).progress.kind,
    "max",
  );
});
test("talent prices use base level and the active depot, not extra levels or another character depot", () => {
  const v = {
    avatar_id: 10000002,
    level: 80,
    exp: 0,
    ascension: 5,
    skill_depot_id: 201,
    skill_levels: { 10018: 7 },
    extra_skill_levels: { 232: 3 },
  };
  const q = nextUpgrades(v, catalog, data()).talents.find(
    (t) => t.skillId === "10018",
  );
  assert.equal(q.from, 7);
  assert.equal(q.to, 8);
  assert.equal(q.mora, 260000);
  assert.equal(q.requiredAscension, 5);
  const max = nextUpgrades(
    { ...v, skill_levels: { 10018: 10 } },
    catalog,
    data(),
  ).talents.find((t) => t.skillId === "10018");
  assert.equal(max.kind, "max");
  assert.deepEqual(
    nextUpgrades({ ...v, skill_depot_id: 501 }, catalog, data()).talents,
    [],
  );
});
test("missing EXP and missing inventory are unknown; explicit zero and stale values remain distinct", () => {
  const d = data();
  assert.equal(observedStock(d, 104001), null);
  assert.equal(observedStock(data({ 104001: 0 }), 104001), 0);
  const v = char(22, 0, 1);
  delete v.props[1001];
  assert.equal(nextUpgrades(v, catalog, d).progress.kind, "unknown");
  const q = nextUpgrades(char(22, 0, 1), catalog, d).progress;
  assert.equal(q.status, "unknown");
  const saved = data({ 104003: 5 });
  saved.domains.find((d) => d.collection === "characters").freshness = "stale";
  assert.equal(
    nextUpgrades(char(22, 0, 1), catalog, saved).progress.status,
    "saved",
  );
  const invalid = char(50, 0, 1);
  assert.equal(nextUpgrades(invalid, catalog, saved).progress.kind, "unknown");
});

test("only explicit complete material coverage proves an absent ID is zero", () => {
  const d=data();
  const domain=d.domains.find(d=>d.collection==='material_stacks');
  domain.scope='full';
  assert.equal(observedStock(d,104003),0);
  assert.equal(observedStock(d,113058),0);
  assert.equal(observedStock(d,202,true),10000000);
  d.collections.wallet=[];
  assert.equal(observedStock(d,202,true),null);
  for(const scope of ['partial','unverified']){domain.scope=scope;assert.equal(observedStock(d,104003),null);}
  domain.scope='full';domain.freshness='stale';assert.equal(observedStock(d,104003),null);
  domain.freshness='error';assert.equal(observedStock(d,104003),null);
  domain.freshness='saved';assert.equal(observedStock(d,104003),0);
  d.collections.material_stacks=[{confirmation:'historical_unconfirmed',values:{item_id:104003,count:24}}];
  assert.equal(observedStock(d,104003),null);
});
