import test from 'node:test';
import assert from 'node:assert/strict';
import {weaponDetails,equipmentStatValue,equipmentTooltip} from '../src/equipment-details.mjs';
const reference={weapons:{15402:{promotionId:15402,props:[{type:'FIGHT_PROP_BASE_ATTACK',initial:42.401,curve:'A'},{type:'FIGHT_PROP_ELEMENT_MASTERY',initial:36,curve:'B'}],effects:{1:{name:'Песнь без стрел',description:'24%'},5:{name:'Песнь без стрел',description:'48%'}}}},promotions:{15402:{5:{maxLevel:80,props:{FIGHT_PROP_BASE_ATTACK:130}},6:{maxLevel:90,props:{FIGHT_PROP_BASE_ATTACK:155.6}}}},curves:{80:{A:{arith:'ARITH_MULTI',value:7.5},B:{arith:'ARITH_MULTI',value:4}},90:{A:{arith:'ARITH_MULTI',value:8.349},B:{arith:'ARITH_MULTI',value:4.594}}},sets:{1:{name:'Комплект A',bonuses:[{pieces:2,description:'Бонус A2'},{pieces:4,description:'Бонус A4'}]},2:{name:'Комплект B',bonuses:[{pieces:2,description:'Бонус B2'}]}},relicSets:{11:1,12:1,13:1,14:1,21:2,22:2}};
const catalog={equipmentDetails:reference};
const artifact=(guid,id)=>({entity_key:guid,source_collection:'artifacts',values:{item_id:id,kind:'artifact'}});
test('real Stringless 90 values are 510 ATK / 165 EM; refinement changes only effect',()=>{
 const v={item_id:15402,level:90,ascension:6,refinement:5};
 const info=weaponDetails(v,catalog);
 assert.deepEqual(info.props.map(equipmentStatValue),['510','165']);assert.equal(info.effect.description,'48%');
 assert.equal(weaponDetails({...v,refinement:1},catalog).effect.description,'24%');
 assert.equal(weaponDetails({...v,ascension:5},catalog).props[0].value,null);
 const before=weaponDetails({...v,level:80,ascension:5},catalog),after=weaponDetails({...v,level:80},catalog);
 assert.ok(Math.abs(after.props[0].value-before.props[0].value-25.6)<1e-8);assert.equal(before.props[1].value,after.props[1].value);
});
test('set tooltips count unique equipped instances: 1, 2+2, 4 and changes after removal',()=>{
 const a=artifact('a',11),b=artifact('b',12),c=artifact('c',13),d=artifact('d',14),x=artifact('x',21),y=artifact('y',22);
 assert.match(equipmentTooltip(a,[a],catalog).description,/Бонус не активен/);
 const split=equipmentTooltip(a,[a,b,x,y],catalog).description;assert.match(split,/Бонус A2/);assert.doesNotMatch(split,/Бонус A4|Бонус B2/);
 assert.match(equipmentTooltip(a,[a,b,c,d],catalog).description,/Бонус A4/);
 assert.doesNotMatch(equipmentTooltip(a,[a,b,c,c],catalog).description,/Бонус A4/);
 assert.doesNotMatch(equipmentTooltip(a,[a,b,c],catalog).description,/Бонус A4/);
});
test('weapon tooltip uses current refinement',()=>{
 const row={source_collection:'weapons',values:{item_id:15402,level:90,ascension:6,refinement:5}};
 assert.equal(equipmentTooltip(row,[row],catalog).description,'48%');
});
