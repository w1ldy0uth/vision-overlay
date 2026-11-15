import test from 'node:test';
import assert from 'node:assert/strict';
import {inventoryWindow,anchorShift} from '../src/virtual-inventory.mjs';
const row=n=>({source_collection:'material_stacks',entity_key:String(n)});
test('continuous windows cover a large collection with bounded rendered rows and aligned grid lines',()=>{
 for(const columns of [1,4,8])for(let offset=0;offset<100000;offset+=123){
  const w=inventoryWindow(816,columns,280,offset,1080);
  assert.equal(w.start%columns,0);assert.ok(w.end<=816);assert.ok(w.end-w.start<=columns*12);
  assert.equal(w.top+w.bottom+Math.ceil((w.end-w.start)/columns)*280,Math.ceil(816/columns)*280);
 }
 assert.deepEqual(inventoryWindow(0,8,280,0,1080),{start:0,end:0,top:0,bottom:0});
});
test('first and last items remain reachable without a page boundary, including partial last lines',()=>{
 assert.equal(inventoryWindow(367,8,260,-500,1000).start,0);
 assert.equal(inventoryWindow(367,8,260,100000,1000).end,367);
 const first=inventoryWindow(10000,1,84,0,900),next=inventoryWindow(10000,1,84,700,900);
 assert.ok(next.start<first.end);assert.ok(next.end>first.end);
});
test('insertions and deletions above the viewport preserve the visible identity; collections do not collide',()=>{
 const old=Array.from({length:100},(_,n)=>row(n));
 assert.equal(anchorShift(old,[...Array.from({length:8},(_,n)=>row(100+n)),...old],40,8),1);
 assert.equal(anchorShift(old,old.slice(8),40,8),-1);
 assert.equal(anchorShift(old,old.filter(r=>r.entity_key!=='40'),40,8),0);
 assert.equal(anchorShift([row(1)],[{...row(1),source_collection:'weapons'}],0,1),0);
});
