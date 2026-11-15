import test from 'node:test';
import assert from 'node:assert/strict';
import {artifactStats,equipmentStatValue} from '../src/equipment-details.mjs';
const catalog={rarities:{42:5},mainProps:[{id:7,propType:'FIGHT_PROP_CRITICAL_HURT'}],relicLevels:[{rank:5,level:20,addProps:[{propType:'FIGHT_PROP_CRITICAL_HURT',value:.59}]},{rank:5,level:21,addProps:[{propType:'FIGHT_PROP_CRITICAL_HURT',value:.622}]}],affixes:[{id:1,propType:'FIGHT_PROP_ATTACK_PERCENT',propValue:.0583},{id:2,propType:'FIGHT_PROP_HP',propValue:298.75}]};
test('artifact preview uses actual rarity and level+1 main stat; repeated rolls are combined',()=>{
 const info=artifactStats({item_id:42,level:20,main_prop_id:7,append_prop_ids:[1,2,1]},catalog);
 assert.equal(equipmentStatValue(info.main),'62,2%');
 assert.deepEqual(info.substats,[{type:'FIGHT_PROP_ATTACK_PERCENT',value:.1166},{type:'FIGHT_PROP_HP',value:298.75}]);
 assert.equal(equipmentStatValue(info.substats[0]),'11,7%');assert.equal(info.unknownSubstats,0);
 const changed=artifactStats({item_id:42,level:19,main_prop_id:7,append_prop_ids:[2]},catalog);
 assert.equal(changed.main.value,.59);assert.equal(changed.substats.length,1);
});
test('missing artifact data remains unknown instead of appearing as zero',()=>{
 const info=artifactStats({item_id:42,level:20,main_prop_id:7,append_prop_ids:[999]}, {...catalog,relicLevels:[]});
 assert.equal(info.main.value,null);assert.equal(info.unknownSubstats,1);assert.deepEqual(info.substats,[]);
});
