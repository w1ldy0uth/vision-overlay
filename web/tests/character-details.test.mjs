import test from 'node:test';
import assert from 'node:assert/strict';
import {characterAbilities} from '../src/character-details.mjs';
const catalog={skillDepots:{1:{skills:[11,12,13,0],energySkill:14,attackModeSkill:20000,talents:[101,102,103,104,105,106]},2:{skills:[21,22],energySkill:23,talents:[201,202]}},characterDetails:{skills:{13:{name:'Спринт'}},passives:{5:{name:'Пассивный'}},depots:{1:{passives:[{groupId:5,requiredAscension:4}]}},constellations:{103:{name:'Третье'}}}};
test('all depot skills include alternate sprint; passives use ascension and C unlocks use exact IDs',()=>{
 const result=characterAbilities({skill_depot_id:1,skill_levels:{11:3,12:4,14:5},talent_ids:[103],props:{1002:{ival:2}}},catalog);
 assert.deepEqual(result.active.map(t=>t.id),['11','12','13','14']);
 assert.equal(result.active[2].name,'Спринт');assert.equal(result.active[2].level,undefined);
 assert.equal(result.passives[0].unlocked,false);
 assert.equal(result.constellations[0].unlocked,false);assert.equal(result.constellations[2].unlocked,true);
 assert.equal(characterAbilities({skill_depot_id:1,ascension:4},catalog).passives[0].unlocked,true);
});
test('switching traveler depot uses separate skills and constellations without leaking previous element',()=>{
 const result=characterAbilities({skill_depot_id:2,skill_levels:{21:5,22:4,23:4},talent_ids:[201]},catalog);
 assert.deepEqual(result.active.map(t=>t.id),['21','22','23']);assert.deepEqual(result.constellations.map(c=>c.id),['201','202']);
 assert.equal(result.constellations[0].unlocked,true);assert.equal(result.passives.length,0);
});

test('reference cards use the catalog depot and contain no captured talent levels or acquired constellations',()=>{
 const result=characterAbilities({avatar_id:999,skill_depot_id:2,skill_levels:{11:10,12:10,14:10},talent_ids:[101,102,103]}, {...catalog,avatarMetadata:{999:{skillDepotId:1}},skillNames:{11:'Атака',12:'Навык',14:'Ульта'}},{referenceOnly:true});
 assert.deepEqual(result.active.map(t=>t.id),['11','12','13','14']);
 assert.ok(result.active.every(t=>t.level===undefined));assert.ok(result.constellations.every(t=>t.unlocked===null));assert.ok(result.passives.every(t=>t.unlocked===null));
});
