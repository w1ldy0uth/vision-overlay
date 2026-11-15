import test from 'node:test';
import assert from 'node:assert/strict';
import {analyzeWishes,classifyFeatured} from '../src/wish-pity.mjs';
const row=(id,rank='3',type='301',name='Акционный')=>({id:String(id),rank_type:rank,gacha_type:type,name,time:'2026-11-15 12:00:00',lang:'ru',item_id:''});
const catalog={avatarNames:{'10000001':'Акционный','10000002':'Стандартный'},names:{'11001':'Меч','11002':'Другой'}};
const periods=[{group:'characters',start:'2026-11-01 18:00:00',end:'2026-11-24 17:59:59',featured:[{name:'Акционный'}]},{group:'weapons',start:'2026-11-01 18:00:00',end:'2026-11-24 17:59:59',featured:[{name:'Меч'}]}];
const coverage={all_available_pages_fetched:true,ranges:{characters:{first:'1',last:'999'},standard:{first:'1',last:'999'},weapons:{first:'1',last:'999'}}};
const options={catalog,periods,coverage,zone:1};
test('301/400 share counters; independent pools and large IDs retain exact order',()=>{
 const base=184467440737095516160000n;
 const rows=[row(base+3n,'3','400'),row(base+1n,'5'),row(base+2n,'3'),row('2','5','200')];
 const v=analyzeWishes(rows,{...options,coverage:{...coverage,ranges:{...coverage.ranges,characters:{first:String(base),last:String(base+4n)}}}});
 assert.equal(v[0].pity,2);assert.equal(v[0].remaining,88);assert.equal(v[0].exact,true);assert.equal(v[2].pity,0);assert.equal(v[2].remaining,90);
 assert.equal(v[2].guarantee,'not_applicable');assert.equal(v[4].cap,null);assert.equal(v[4].guarantee,'not_applicable');
});
test('off-banner gives a featured guarantee; a featured drop clears it and is not a claimed exact 50/50 probability',()=>{
 assert.equal(analyzeWishes([row(1,'5','301','Стандартный'),row(2)],options)[0].guarantee,'guaranteed');
 assert.equal(analyzeWishes([row(1,'5','301','Стандартный'),row(2,'5','400')],options)[0].guarantee,'not_guaranteed');
 const w=analyzeWishes([row(1,'5','302','Другой'),row(2,'3','302')],options)[1];
 assert.equal(w.guarantee,'guaranteed');assert.equal(w.remaining,79);assert.equal(w.path,'unknown');
});
test('no anchor, missing import coverage, gaps and impossible counters never invent a remaining count',()=>{
 const missing=analyzeWishes([row(1),row(2)],options)[0];assert.equal(missing.pity,2);assert.equal(missing.exact,false);assert.equal(missing.remaining,null);
 const gap=analyzeWishes([row(1,'5'),row(2)],{...options,coverage:{...coverage,ranges:{characters:{first:'2',last:'3'}}}})[0];assert.equal(gap.exact,false);assert.equal(gap.guarantee,'unknown');
 const impossible=analyzeWishes([row(1,'5'),...Array.from({length:90},(_,i)=>row(i+2))],options)[0];assert.equal(impossible.invalid,true);assert.equal(impossible.remaining,null);
 assert.equal(analyzeWishes([],options)[0].guarantee,'unknown');
});
test('unknown, overlapping, untranslated or missing banner metadata cannot imply a loss',()=>{
 const r=row(1,'5');assert.equal(classifyFeatured(r,'characters',catalog,[],1),null);
 assert.equal(classifyFeatured(r,'characters',catalog,[...periods,periods[0]],1),null);
 assert.equal(classifyFeatured({...r,name:'Неизвестный'},'characters',catalog,periods,1),null);
 assert.equal(classifyFeatured(r,'characters',catalog,[{...periods[0],featured:[{name:null}]}],1),null);
 assert.equal(classifyFeatured({...r,lang:'en-us'},'characters',catalog,periods,1),null);
 assert.equal(classifyFeatured({...r,lang:'en-us',item_id:'10000001'},'characters',catalog,periods,1),true);
});
test('global version starts use server zone while ordinary starts and ends stay server-local',()=>{
 const global=[{...periods[0],start:'2026-11-04 06:00:00',startGlobal:true}];
 const r={...row(1,'5'),time:'2026-11-03 23:30:00'};
 assert.equal(classifyFeatured(r,'characters',catalog,global,1),true);
 assert.equal(classifyFeatured(r,'characters',catalog,global,8),null);
 assert.equal(classifyFeatured(r,'characters',catalog,global,null),null);
 assert.equal(classifyFeatured({...r,time:'2026-11-08 12:00:00'},'characters',catalog,global,null),true);
 assert.equal(classifyFeatured({...r,time:'2026-11-24 18:00:00'},'characters',catalog,global,1),null);
});
test('chronicled path is unknown even after a 5-star; first historical cycle is a lower bound',()=>{
 const s=analyzeWishes([row(1),row(2,'5'),row(3),row(4,'5'),row(5,'5','500')],options);
 assert.equal(s[3].guarantee,'unknown');assert.deepEqual(s[0].drops.map(x=>[x.count,x.exact]),[[2,false],[2,true]]);
});
