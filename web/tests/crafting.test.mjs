import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import {craftFromStock} from '../src/crafting.mjs';
import {allocateGoals} from '../src/allocation.mjs';
const recipes={
 104122:{recipeId:15004,inputId:104121,inputCount:3,outputCount:1,mora:300,requiredAdventureRank:25},
 104123:{recipeId:15005,inputId:104122,inputCount:3,outputCount:1,mora:900,requiredAdventureRank:35},
};
function craft(want,stock,rank=60) {
 const state=new Map(Object.entries(stock).map(([id,n])=>[Number(id),n]));
 const result=craftFromStock(104123,want,recipes,rank,id=>state.get(id)??0,(id,n)=>state.set(id,n));
 return {state,result};
}
test('recursive tier upgrade consumes nine shards once and charges all intermediate crafts',()=>{
 const {state,result:r}=craft(2,{104121:9,202:0});
 assert.equal(r.count,1);assert.equal(r.mora,1800);assert.equal(state.get(104121),0);assert.equal(state.get(202),0);
 assert.deepEqual(r.inputs,[{id:104121,count:9}]);assert.deepEqual(r.steps.map(s=>[s.itemId,s.count]),[[104122,3],[104123,1]]);
});
test('partial intermediate stock is used first, and an incomplete batch leaves stock available',()=>{
 const r=craft(1,{104122:2,104121:3});assert.equal(r.result.count,1);assert.equal(r.result.mora,1200);
 const insufficient=craft(1,{104122:2,104121:2});assert.equal(insufficient.result.count,0);assert.equal(insufficient.state.get(104121),2);assert.equal(insufficient.state.get(104122),2);
});
test('unknown stocks, missing rank and rank locks cannot promise a craft; no random rewards are produced',()=>{
 assert.equal(craft(1,{104122:null}).result.count,0);
 assert.equal(craft(1,{104121:9},34).result.count,0);
 assert.equal(craft(1,{104121:9},null).result.count,0);
 assert.equal(craft(1,{104121:9},35).result.count,1);
});
const fixture=JSON.parse(fs.readFileSync(new URL('./fixtures/leveling.json',import.meta.url)));
function setup(materials=[{id:104123,count:1}],money=50000) {
 const catalog=structuredClone(fixture);catalog.crafting={schemaVersion:1,recipes};catalog.progression.avatarPromotions['137']['1'].materials=materials;
 const goals=['1','2'].map((goal_id,priority)=>({goal_id,values:{collection:'characters',entity_key:goal_id,skill_depot_id:13701,level:20,ascension:1,talents:{},priority}}));
 const data={collections:{characters:goals.map(g=>({entity_key:g.goal_id,confirmation:'observed',values:{avatar_id:10000137,skill_depot_id:13701,level:20,exp:0,ascension:0}})),adventure:[{values:{rank:60}}],material_stacks:[{confirmation:'observed',values:{item_id:104121,count:9}}],wallet:[{confirmation:'observed',values:{item_id:202,count:money}}]},domains:['characters','material_stacks','wallet'].map(collection=>({collection,scope:'full',freshness:'synchronized'}))};
 return {catalog,goals,data};
}
test('crafted supply is separate from owned supply, consumes mora and blocks a competing goal',()=>{
 const {catalog,goals,data}=setup(),before=JSON.stringify({catalog,goals,data});
 const r=allocateGoals(goals,catalog,data);assert.equal(r.plans[0].materials[0].allocated,0);assert.equal(r.plans[0].materials[0].crafted,1);assert.equal(r.plans[0].materials[0].missing,0);
 assert.equal(r.plans[0].mora.count,21800);assert.equal(r.plans[1].materials[0].crafted,0);assert.equal(r.plans[1].materials[0].missing,1);assert.equal(r.craftInputs[0].count,9);
 assert.equal(JSON.stringify({catalog,goals,data}),before);
 const next=allocateGoals([...goals].map(g=>({...g,values:{...g.values,priority:1-g.values.priority}})),catalog,data);assert.equal(next.plans[0].goal.goal_id,'2');assert.equal(next.plans[0].materials[0].crafted,1);
});
test('all direct requirements of a goal are reserved before its raw materials may be crafted',()=>{
 const {catalog,goals,data}=setup([{id:104123,count:1},{id:104121,count:3}]);
 const r=allocateGoals([goals[0]],catalog,data);assert.equal(r.plans[0].materials[0].crafted,0);assert.equal(r.plans[0].materials[1].allocated,3);assert.equal(r.craftInputs.length,0);
});
test('crafting is quoted with an insufficient or unknown wallet, and adds to the mora requirement',()=>{
 for(const amount of [20000,21799,null]) {
  const {catalog,goals,data}=setup(undefined,amount);const r=allocateGoals([goals[0]],catalog,data);
  assert.equal(r.plans[0].materials[0].crafted,1);assert.equal(r.craftInputs[0].count,9);
  assert.equal(r.plans[0].mora.count,21800);
  assert.equal(r.plans[0].mora.missing,amount===null?null:21800-amount);
  assert.equal(r.plans[0].ready,false);
 }
});
test('real crafting events replace the virtual supply without double-counting and old catalogs stay readable',()=>{
 const {catalog,goals,data}=setup();const before=allocateGoals([goals[0]],catalog,data);
 data.collections.material_stacks=[{confirmation:'observed',values:{item_id:104123,count:1}}];data.collections.wallet[0].values.count-=1800;
 const after=allocateGoals([goals[0]],catalog,data);assert.equal(before.plans[0].materials[0].crafted,1);assert.equal(after.plans[0].materials[0].allocated,1);assert.equal(after.plans[0].materials[0].crafted,0);assert.equal(after.plans[0].mora.count,20000);
 delete catalog.crafting;assert.doesNotThrow(()=>allocateGoals(goals,catalog,data));
});

test('every goal gets its direct stock before any goal can use the surplus to craft',()=>{
 const {catalog,goals,data}=setup([{id:104123,count:1},{id:104121,count:3}]);
 data.collections.material_stacks[0].values.count=12;
 const result=allocateGoals(goals,catalog,data);
 const ingredient=result.materials.find(row=>row.id===104121);
 assert.deepEqual(ingredient,{id:104121,count:6,owned:12,allocated:6,crafted:0,craftInput:0,missing:0});
 assert.equal(result.materials.find(row=>row.id===104123).missing,2);
});

test('three goals fill four recipe tiers in order, leaving only the highest tier short',()=>{
 const requirements=[104124,104123,104122,104121].map(id=>({id,count:1}));
 const {catalog,goals,data}=setup(requirements,0);
 catalog.crafting.recipes={...recipes,104124:{recipeId:15006,inputId:104123,inputCount:3,outputCount:1,mora:2700,requiredAdventureRank:45}};
 goals.push({goal_id:'3',values:{...goals[0].values,priority:2,entity_key:'3'}});
 data.collections.characters.push({...data.collections.characters[0],entity_key:'3'});
 data.collections.material_stacks[0].values.count=65;
 data.collections.material_stacks.push({confirmation:'observed',values:{item_id:104122,count:1}});

 const result=allocateGoals(goals,catalog,data);
 const row=id=>result.materials.find(material=>material.id===id);
 assert.deepEqual([104121,104122,104123,104124].map(id=>[row(id).crafted,row(id).missing]),[[0,0],[2,0],[3,0],[1,2]]);
 assert.equal(row(104121).craftInput,60);
 assert.equal(row(104121).count,63);
 assert.equal(row(202).missing,row(202).count);
 assert.equal(row(202).count,74100);
});
