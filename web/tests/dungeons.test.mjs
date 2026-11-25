import { readFileSync } from 'node:fs';
import assert from 'node:assert/strict';
import test from 'node:test';
import { dungeonNeeds, dungeonsForDay } from '../src/dungeons.mjs';
import { nextServerWeekReset, serverDay } from '../src/server-clock.mjs';

const { dungeons, trounce } = JSON.parse(readFileSync(new URL('../src/dungeon-schedule.json', import.meta.url)));

test('the domain rotation rolls over at 04:00 on the selected game server', () => {
  const reset = Date.parse('2026-11-29T03:00:00Z');
  assert.deepEqual(serverDay('os_euro', reset - 1), { weekday: 6, nextReset: reset });
  assert.deepEqual(serverDay('os_euro', reset), { weekday: 0, nextReset: reset + 86_400_000 });
  assert.equal(serverDay('os_asia', reset - 1).weekday, 0);
  assert.equal(serverDay('os_cht', reset - 1).weekday, 0);
  assert.equal(serverDay('os_usa', reset).weekday, 6);
});

test('weekday and Sunday rewards use the domain rotation including Snezhnaya', () => {
  const wednesday = dungeonsForDay(dungeons, 3);
  assert.equal(wednesday.length, 16);
  assert(wednesday.every(entry => entry.variants.length === 1));
  assert.deepEqual(wednesday, dungeonsForDay(dungeons, 6));
  const sunday = dungeonsForDay(dungeons, 0);
  assert(sunday.every(entry => entry.variants.length === 3));
  assert.deepEqual(wednesday.find(entry => entry.id === 2074).variants[0].rewards, [104371, 104372, 104373]);
});

test('weekly boss rewards reset on Monday on the selected server', () => {
  const reset = Date.parse('2026-11-30T03:00:00Z');
  assert.equal(nextServerWeekReset('os_euro', reset - 1), reset);
  assert.equal(nextServerWeekReset('os_euro', reset), reset + 7 * 86_400_000);
  assert.equal(nextServerWeekReset('os_usa', reset), Date.parse('2026-11-30T09:00:00Z'));
  assert.equal(nextServerWeekReset('os_asia', reset), Date.parse('2026-12-06T20:00:00Z'));
});

test('weekly rewards include Andrius and all three drops of each domain', () => {
  assert.equal(trounce.length, 14);
  assert(trounce.every(dungeon => dungeon.rewards.length === 3));
  assert.equal(new Set(trounce.flatMap(dungeon => dungeon.rewards)).size, 42);
  assert.deepEqual(trounce.find(dungeon => dungeon.name === 'Андриус').rewards, [113006, 113007, 113008]);
});

test('domain matches reflect shortages after crafting and keep unknown stocks visible', () => {
  const plans = [{ goal: { goal_id: 'character' }, plan: { values: { avatar_id: 10000023 } }, materials: [
    { id: 104371, missing: 0, crafted: 5 },
    { id: 104372, missing: 2 },
    { id: 104373, missing: null },
    { id: 112124, missing: 8 },
  ] }];
  const result = dungeonNeeds(plans, new Set([104371, 104372, 104373]));
  assert.deepEqual([...result].map(([id, goals]) => [id, goals[0].goalId, goals[0].missing]), [
    [104372, 'character', 2], [104373, 'character', null],
  ]);
});
