import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {allocateGoals} from '../src/allocation.mjs';
const catalog=JSON.parse(readFileSync(new URL('./fixtures/leveling.json',import.meta.url)));
const goal=(key,priority=0,level=20,ascension=1)=>({goal_id:key,values:{collection:'characters',entity_key:key,skill_depot_id:13701,level,ascension,talents:{},priority}});
const character=(key,level=20,exp=0,ascension=0)=>({entity_key:key,confirmation:'observed',values:{avatar_id:10000137,skill_depot_id:13701,level,exp,ascension,skill_levels:{30601:1,30602:1,30605:1}}});
function data(stocks={},characters=[character('1'),character('2')],mora=25000) {
 return {collections:{characters,material_stacks:Object.entries(stocks).map(([item_id,count])=>({confirmation:'observed',values:{item_id:Number(item_id),count}})),wallet:[{confirmation:'observed',values:{item_id:202,count:mora}}]},domains:['characters','material_stacks','wallet'].map(collection=>({collection,freshness:'synchronized',scope:'full'}))};
}
const resource=(p,id)=>p.materials.find(m=>m.id===id);
test('shared ascension materials and mora are consumed once; priority changes recipients, not inventory',()=>{
 const d=data({101273:4,104141:2,112146:6}),goals=[goal('1',10),goal('2',0)];
 const before=JSON.stringify({d,goals,catalog});const result=allocateGoals(goals,catalog,d);
 assert.deepEqual(result.plans.map(p=>p.goal.goal_id),['2','1']);
 assert.equal(resource(result.plans[0],101273).allocated,3);assert.equal(resource(result.plans[1],101273).allocated,1);
 assert.equal(resource(result,101273).missing,2);assert.equal(resource(result,101273).allocated,4);
 assert.equal(result.plans[0].mora.allocated,20000);assert.equal(result.plans[1].mora.allocated,5000);assert.equal(result.plans[1].mora.missing,15000);
 assert.equal(JSON.stringify({d,goals,catalog}),before);
 // Reservations are derived again after persisted goals/facts are loaded.
 const restored=JSON.parse(before);
 assert.deepEqual(allocateGoals(restored.goals,restored.catalog,restored.d),result);
 const swapped=allocateGoals([goal('1',0),goal('2',10)],catalog,d);
 assert.equal(resource(swapped.plans[0],101273).allocated,3);assert.equal(swapped.plans[0].goal.goal_id,'1');
 assert.deepEqual(resource(swapped,101273),resource(result,101273));
});
test('alternate EXP materials satisfy a goal and the next goal cannot use the same book',()=>{
 const d=data({104003:1},[character('1',22,4425,1),character('2',22,4425,1)],10000);
 const result=allocateGoals([goal('2',1,23),goal('1',0,23)],catalog,d);
 const first=result.plans[0],second=result.plans[1];
 assert.equal(first.plan.experience,13575);assert.equal(resource(first,104003).allocated,1);assert.equal(first.mora.count,4000);assert.equal(first.ready,true);
 assert.equal(second.experienceMissing,13575);assert.equal(second.ready,false);
 assert.equal(resource(result,104003).allocated,1);assert.ok(second.materials.every(m=>m.allocated===0));
});
test('partial EXP supplies are reserved and only the outstanding EXP needs future books',()=>{
 const result=allocateGoals([goal('1',0,23)],catalog,data({104002:1},[character('1',22,4425,1)],10000));
 const p=result.plans[0];assert.equal(resource(p,104002).allocated,1);assert.equal(p.experienceMissing,8575);
 assert.equal(p.mora.count,2800);assert.equal(p.mora.missing,0);assert.equal(resource(p,104002).count,2);
 assert.equal(resource(p,104001).missing,4);
});
test('picking up, spending externally, completing and removing goals each rebuild reservations',()=>{
 const goals=[goal('1'),goal('2',1)];
 assert.equal(resource(allocateGoals(goals,catalog,data({101273:4})).plans[1],101273).allocated,1);
 assert.equal(resource(allocateGoals(goals,catalog,data({101273:6})).plans[1],101273).allocated,3);
 assert.equal(resource(allocateGoals(goals,catalog,data({101273:2})).plans[1],101273).allocated,0);
 const completed=data({101273:3},[character('1',20,0,1),character('2')]);
 const result=allocateGoals(goals,catalog,completed);assert.equal(result.plans[0].plan.complete,true);assert.equal(resource(result.plans[1],101273).allocated,3);
 assert.equal(resource(allocateGoals([goal('2')],catalog,data({101273:3})).plans[0],101273).allocated,3);
});
test('unknown inventory stays unknown, while a known sufficient EXP subset can still satisfy a goal',()=>{
 const d=data();d.domains.find(d=>d.collection==='material_stacks').scope='partial';
 const p=allocateGoals([goal('1')],catalog,d).plans[0];assert.equal(resource(p,101273).allocated,null);assert.equal(resource(p,101273).missing,null);assert.equal(p.ready,false);
 const xp=data({104003:1},[character('1',22,4425,1)],10000);xp.domains.find(d=>d.collection==='material_stacks').scope='partial';
 assert.equal(allocateGoals([goal('1',0,23)],catalog,xp).plans[0].ready,true);
 xp.collections.material_stacks=[];
 const unknown=allocateGoals([goal('1',0,23)],catalog,xp).plans[0];assert.equal(unknown.experienceMissing,null);assert.ok(unknown.materials.every(m=>m.missing===null));
});
test('equal priorities are deterministic; missing or another profile entities reserve nothing',()=>{
 const goals=[goal('2'),goal('1')],d=data({101273:3});
 assert.deepEqual(allocateGoals(goals,catalog,d),allocateGoals([...goals].reverse(),catalog,d));
 const other=allocateGoals(goals,catalog,data({},[]));assert.equal(other.partial,true);assert.equal(other.materials.length,0);
});
test('no resource or currency can be allocated above the stock across many competing goals',()=>{
 for(let stock=0;stock<20;stock++) {
  const goals=Array.from({length:5},(_,i)=>goal(String(i+1),i%3));
  const result=allocateGoals(goals,catalog,data({101273:stock,104141:stock,112146:stock},goals.map(g=>character(g.goal_id)),stock*5000));
  for(const m of result.materials)assert.ok(m.allocated>=0&&m.allocated<=m.owned,JSON.stringify(m));
 }
});
test('weapon ores share a bounded supply between weapons and the same mora pool with characters',()=>{
 const d=data({104013:1,104003:1},[character('1',22,4425,1)],4500);
 d.collections.weapons=['3','4'].map(entity_key=>({entity_key,confirmation:'observed',values:{item_id:11101,level:1,exp:0,ascension:0}}));
 d.domains.push({collection:'weapons',freshness:'synchronized',scope:'full'});
 const weapon=(key,priority)=>({goal_id:key,values:{collection:'weapons',entity_key:key,level:2,ascension:0,skill_depot_id:null,talents:{},priority}});
 const result=allocateGoals([goal('1',1,23),weapon('3',0),weapon('4',2)],catalog,d);
 assert.equal(resource(result.plans[0],104013).allocated,1);assert.equal(result.plans[0].mora.count,1000);
 assert.equal(result.plans[1].mora.allocated,3500);assert.equal(result.plans[1].mora.missing,500);
 assert.ok(result.plans[2].materials.every(m=>m.allocated===0));
 assert.equal(resource(result,104013).allocated,1);assert.equal(resource(result,202).allocated,4500);
});

test('level checkpoints distinguish the same level before and after ascension', async ()=>{
 const {levelTargets,targetIndex}=await import('../src/goal-targets.mjs');
 const targets=levelTargets();
 const i=targetIndex(targets,{level:70,ascension:4});
 assert.deepEqual(targets.slice(i-1,i+3),[
  {level:69,ascension:4},{level:70,ascension:4},{level:70,ascension:5},{level:71,ascension:5},
 ]);
 assert.deepEqual(levelTargets(70).at(-1),{level:70,ascension:4});
});

test('equipment swaps retarget the plan; standalone and linked goals reserve each weapon once',()=>{
 const d=data({104013:5},[character('1')],50000);
 const hero=d.collections.characters[0];
 hero.values.equip_guids=['3'];
 d.collections.weapons=['3','4'].map(entity_key=>({entity_key,confirmation:'observed',values:{item_id:11101,level:1,exp:0,ascension:0}}));
 d.domains.push({collection:'weapons',freshness:'synchronized',scope:'full'});
 const linked=goal('1',0,20,0);
 linked.values.equipped_weapon={level:2,ascension:0};
 const standalone={goal_id:'3',values:{collection:'weapons',entity_key:'3',level:1,ascension:0,talents:{},priority:1}};
 const first=allocateGoals([linked,standalone],catalog,d);
 assert.equal(first.plans.length,2);
 assert.equal(first.plans.find(p=>p.goal.goal_id==='3').plan.targetLevel,2);
 assert.equal(standalone.values.level,1);
 hero.values.equip_guids=['4'];
 const swapped=allocateGoals([linked,standalone],catalog,d);
 assert.equal(swapped.plans.find(p=>p.goal.goal_id==='3').plan.complete,true);
 assert.equal(swapped.plans.find(p=>p.goal.goal_id==='1:weapon').plan.targetLevel,2);
 hero.values.equip_guids=[];
 assert.equal(allocateGoals([linked],catalog,d).plans.find(p=>p.goal.goal_id==='1:weapon').plan.status,'unknown');
});
