import test from 'node:test';
import assert from 'node:assert/strict';
import {equipmentOwners} from '../src/equipment-owners.mjs';
const avatar=(id,equipment,extra={})=>({entity_key:id,values:{avatar_id:id,equip_guids:equipment,...extra}});
test('ownership uses instance GUIDs and rebuilds after transfer and removal',()=>{
 const a=avatar('a',['weapon-1','artifact-1']),b=avatar('b',['weapon-2']);
 assert.equal(equipmentOwners([a,b]).get('weapon-1'),a);
 assert.equal(equipmentOwners([a,b]).get('weapon-2'),b);
 const nextA=avatar('a',[]),nextB=avatar('b',['weapon-1','weapon-2']);
 const owners=equipmentOwners([nextA,nextB]);
 assert.equal(owners.get('weapon-1'),nextB);
 assert.equal(owners.has('artifact-1'),false);
});
test('trial and ambiguous ownership never invent an owner',()=>{
 const a=avatar('a',['x','x']),b=avatar('b',['x']),trial=avatar('trial',['y'],{trial_avatar_info:{}});
 assert.equal(equipmentOwners([a]).get('x'),a);
 assert.equal(equipmentOwners([a,b,trial]).get('x'),null);
 assert.equal(equipmentOwners([a,b,trial]).has('y'),false);
});
