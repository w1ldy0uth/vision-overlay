import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import assert from 'node:assert/strict';

const [input, textMaps, revision, output] = process.argv.slice(2);
assert(input && textMaps && /^[a-f0-9]{40}$/.test(revision ?? '') && output,
  'Usage: node scripts/build-dungeon-schedule.mjs TABLES TEXT_MAPS REVISION OUTPUT');
const read = (root, name) => JSON.parse(readFileSync(join(root, name), 'utf8'));
const table = name => read(input, `${name}.json`);
const texts = Object.assign({}, ...readdirSync(textMaps)
  .filter(name => /^TextMap(?:_Medium)?RU(?:_\d+)?\.json$/.test(name))
  .map(name => read(textMaps, name)));
const localized = new Set(Object.values(texts));
const names = {
  44: 'Забытый каньон', 50: 'Сад Сесилии', 131: 'Тайшаньфу', 135: 'Лабиринт Ляншань',
  310: 'Фиалковый зал', 368: 'Зал текущего песка', 505: 'Башня невежества', 509: 'Башня презренной гордости',
  865: 'Бледный забытый цветок', 859: 'Эхо глубоких приливов', 1186: 'Огненные руины', 1188: 'Древний дозорный пункт',
  1690: 'Столица непроглядной тьмы', 1694: 'Заброшенный лунный двор', 2074: 'Заброшенные святыни', 2106: 'Шрамы одержимости',
};
const daily = new Map(table('DailyDungeonConfigData').map(row => [row.id, row]));
const challenges = new Map(table('DungeonExcelConfigData').map(row => [row.id, row]));
const rewards = new Map(table('RewardPreviewExcelConfigData').map(row => [row.id, row.previewItems]));
const points = table('scene3_point').points;
const idsFor = (rotation, day) => [...new Set(rotation.flatMap(row => row[day]))].sort((a, b) => a - b);
const sortIds = ids => [...new Set(ids)].sort((a, b) => a - b);
const types = { DUNGEN_ENTRY_TYPE_AVATAR_TALENT: 'talent', DUNGEN_ENTRY_TYPE_WEAPON_PROMOTE: 'weapon' };
const dungeons = table('DungeonEntryExcelConfigData')
  .filter(entry => entry.isShowInAdvHandbook && entry.isDailyRefresh && types[entry.type])
  .map(entry => {
    const id = entry.dungeonEntryId;
    assert(localized.has(names[id]), `Missing localized dungeon name for ${id}`);
    const rotation = points[id].dungeonRandomList.map(listId => daily.get(listId));
    const groups = [
      ['monday', 'thursday', [1, 4, 0]],
      ['tuesday', 'friday', [2, 5, 0]],
      ['wednesday', 'CHGDPBMNDEF', [3, 6, 0]],
    ];
    const variants = groups.map(([day, pairedDay, days], index) => {
      const ids = idsFor(rotation, day);
      assert.deepEqual(ids, idsFor(rotation, pairedDay));
      const stages = ids.map(challengeId => challenges.get(challengeId));
      const label = texts[stages[0].displayNameTextMapHash];
      assert(label && stages.every(stage => texts[stage.displayNameTextMapHash] === label));
      const expected = sortIds(entry.descriptionCycleRewardList[index]);
      const actual = sortIds(stages.flatMap(stage => rewards.get(stage.passRewardPreviewID)
        .map(item => item.id).filter(itemId => expected.includes(itemId))));
      assert.deepEqual(actual, expected, `Reward mismatch for ${id} on ${day}`);
      return { label, days, rewards: actual };
    });
    assert.deepEqual(sortIds(groups.flatMap(([day]) => idsFor(rotation, day))), idsFor(rotation, 'sunday'));
    const first = challenges.get(idsFor(rotation, 'monday')[0]);
    assert(first.statueCostID === 106 && first.statueCostCount > 0);
    return { id, name: names[id], region: first.cityID, kind: types[entry.type], resin: first.statueCostCount, variants };
  })
  .sort((a, b) => a.region - b.region || a.kind.localeCompare(b.kind));

const weeklyNames = {
  38: 'Логово Ужаса Бури', 218: 'Золотая палата', 271: 'Под древом подавления',
  396: 'Остров Наруками: Тэнсюкаку', 503: 'Конец Царства онейроса', 731: 'Мастерская Дзёрури',
  848: 'Владение начинаний', 1085: 'Тень другого мира', 1241: 'Разрозненные руины',
  1488: 'Записи на каменной стеле', 1576: 'Недоигранная партия в шахматы',
  1919: 'Институт ложной луны', 2053: 'Оковы вселенской нирваны',
};
const talentDrops = previewIds => sortIds(previewIds.flatMap(id => (rewards.get(id) ?? [])
  .map(reward => reward.id).filter(id => id >= 113000 && id < 114000 && id !== 113021)));
const trounce = table('DungeonEntryExcelConfigData')
  .filter(entry => entry.isShowInAdvHandbook && entry.type === 'DUNGEON_ENTRY_TYPE_TRIAL')
  .map(entry => {
    const id = entry.dungeonEntryId;
    const name = weeklyNames[id];
    assert(localized.has(name), `Missing localized trounce domain name for ${id}`);
    const stages = (id === 271
      ? [...challenges.values()].filter(stage => stage.sceneId === 20106)
      : points[id].dungeonIds.map(stageId => challenges.get(stageId)))
      .filter(stage => stage?.stateType === 'DUNGEON_STATE_RELEASE' && stage.subType === 'DUNGEON_SUB_BOSS');
    const drops = talentDrops(stages.map(stage => stage.passRewardPreviewID));
    assert.equal(drops.length, 3, `Invalid talent drops for ${name}`);
    return { id, name, region: stages[0].cityID, rewards: drops };
  });
// Andrius has a weekly Trounce Blossom in the open world and no dungeon entry.
const wolfDrops = talentDrops([15010]);
assert.deepEqual(wolfDrops, [113006, 113007, 113008]);
trounce.push({ id: 15010, name: 'Андриус', region: 1, location: 'Вольфендом', rewards: wolfDrops });
trounce.sort((a, b) => a.region - b.region || a.id - b.id);

const rows = entries => entries.map(entry => `    ${JSON.stringify(entry)}`).join(',\n');
writeFileSync(output, `{"revision":"${revision}","dungeons":[\n${rows(dungeons)}\n],"trounce":[\n${rows(trounce)}\n]}\n`);
