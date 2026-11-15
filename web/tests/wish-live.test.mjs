import test from 'node:test';
import assert from 'node:assert/strict';
import {wishRecords} from '../src/wish-live.mjs';
import {analyzeWishes} from '../src/wish-pity.mjs';
const coverage={all_available_pages_fetched:true,region_time_zone:1,ranges:{standard:{first:'1',last:'1'}}};
const api={id:'1',rank_type:'5',gacha_type:'200',name:'Меч',time:'2026-11-15 12:00:00'};
const capture=(live_id,index=0,item_id='15302')=>({live_id,result_index:index,source_sequence:'25',item_id,gacha_type:'200',anchor_id:'1',observed_ms:String(Date.parse('2026-11-15T12:05:00Z'))});
const catalog={names:{15302:'Клятва стрелка',11501:'Меч'},rarities:{15302:3,11501:5}};
test('local identities do not become API IDs and only enabled observations enter history',()=>{
 const data={records:[api],live_records:[capture('uuid-a')],live_enabled:true,last_import:{metadata:coverage}};
 const rows=wishRecords(data,catalog);
 assert.equal(rows.length,2);assert.equal(rows[0].id,'live:uuid-a');assert.equal(rows[0].name,'Клятва стрелка');assert.equal(rows[0].rank_type,'3');assert.equal(rows[0].time,'2026-11-15 13:05:00');
 assert.equal(wishRecords({...data,live_enabled:false},catalog).length,1);
 const stats=analyzeWishes(rows,{coverage})[2];assert.equal(stats.pity,1);assert.equal(stats.exact,true);
});
test('ten same-second results retain packet order, 5-star resets, and an API prefix does not double-count',()=>{
 const data={records:[api],live_enabled:true,last_import:{metadata:coverage},live_records:Array.from({length:10},(_,i)=>capture(`uuid-${9-i}`,i,i===4?'11501':'15302')).reverse()};
 const rows=wishRecords(data,catalog);const stats=analyzeWishes(rows,{coverage})[2];
 assert.equal(stats.total,11);assert.equal(stats.pity,5);assert.equal(stats.exact,true);assert.equal(stats.last.id,'live:uuid-5');
 const prefix=[api,...rows.filter(r=>r.live&&r.result_index<3).map((r,i)=>({...r,id:String(i+2),live:false}))];
 const after=wishRecords({...data,records:prefix,live_records:data.live_records.filter(r=>r.result_index>=3)},catalog);
 assert.equal(analyzeWishes(after,{coverage})[2].pity,5);assert.equal(after.length,11);
});
test('unsupported rarity is unknown and never guessed as a three-star or a five-star',()=>{
 const rows=wishRecords({records:[],live_enabled:true,live_records:[capture('new',0,'123456')],last_import:null},{});
 assert.equal(rows[0].rank_type,'');assert.equal(analyzeWishes(rows)[2].exact,false);
});
