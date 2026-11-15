import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {planGoal,summarizePlans} from '../src/planner.mjs';
const catalog=JSON.parse(readFileSync(new URL('./fixtures/leveling.json',import.meta.url)));
const goal={collection:'characters',entity_key:'42',skill_depot_id:13701,level:23,ascension:1,talents:{},priority:0};
function data(level=22,exp=4425,asc=1) {return {collections:{characters:[{confirmation:'observed',entity_key:'42',values:{avatar_id:10000137,skill_depot_id:13701,props:{4001:{ival:String(level)},1001:{ival:String(exp)},1002:{ival:String(asc)}},skill_levels:{30601:1,30602:1,30605:1}}}],material_stacks:[],wallet:[{confirmation:'observed',values:{item_id:202,count:'10000'}}]},domains:['characters','wallet','material_stacks'].map(collection=>({collection,freshness:'synchronized'}))};}
test('goal uses remaining EXP and recalculates to completion without mutating targets or game facts',()=>{
 const d=data(),before=JSON.stringify(d),g=JSON.stringify(goal);const p=planGoal(goal,catalog,d);
 assert.equal(p.experience,13575);assert.equal(p.mora,2800);assert.equal(p.complete,false);assert.equal(p.status,'active');
 assert.equal(planGoal(goal,catalog,data(23,0)).complete,true);assert.equal(JSON.stringify(d),before);assert.equal(JSON.stringify(goal),g);
});
test('ascension costs and talent requirements are included, talent bonuses do not change base cost',()=>{
 const d=data(20,0,0);const p=planGoal({...goal,level:20,ascension:1},catalog,d);
 assert.equal(p.experience,0);assert.equal(p.mora,20000);assert.ok(p.materials.length>0);
 const talent=planGoal({...goal,level:22,talents:{30601:2}},catalog,data());
 assert.ok(talent.targetAsc>=2);assert.ok(talent.targetLevel>=40);assert.ok(talent.mora>20000);
 const boosted=data();boosted.collections.characters[0].values.extra_skill_levels={30601:3};
 const q=planGoal({...goal,level:22,talents:{30601:2}},catalog,boosted);assert.deepEqual(q.materials,talent.materials);assert.equal(q.mora,talent.mora);assert.equal(q.experience,talent.experience);
});
test('unknown stock remains unknown, totals sum needs before subtracting shared stock',()=>{
 const d=data();const p=planGoal(goal,catalog,d);assert.equal(p.materials[0].owned,null);
 const id=p.materials[0].id;d.collections.material_stacks=[{confirmation:'observed',values:{item_id:id,count:1}}];
 const total=summarizePlans([p,p],d);const m=total.materials.find(m=>m.id===id);
 assert.equal(m.count,p.materials[0].count*2);assert.equal(m.missing,Math.max(0,m.count-1));
 assert.equal(summarizePlans([p,{status:'unknown'}],d).partial,true);
});
test('missing entity and impossible paths cannot produce a free plan; saved facts stay marked saved',()=>{
 assert.equal(planGoal({...goal,entity_key:'nope'},catalog,data()).status,'unknown');
 assert.equal(planGoal({...goal,level:99},catalog,data()).status,'unknown');
 const d=data();d.domains[0].freshness='stale';assert.equal(planGoal(goal,catalog,d).status,'saved');
});
