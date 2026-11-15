import test from 'node:test';
import assert from 'node:assert/strict';
import {selectedProfile,gameSession} from '../src/profile-selection.mjs';
const state=(active='a',session='capture:1')=>({active_game_profile_id:active,active_game_session:session,profiles:[{profile_id:'a'},{profile_id:'b'}]});
test('initial selection follows the game and an explicit saved-profile choice survives ordinary updates',()=>{
 assert.equal(selectedProfile({profileId:'b',manualSession:null},null),'b');
 assert.equal(selectedProfile({profileId:'b',manualSession:null},state()),'a');
 const manual={profileId:'b',manualSession:gameSession(state())};
 assert.equal(selectedProfile(manual,{...state(),revision:'999'}),'b');
 assert.equal(selectedProfile(manual,{...state(),revision:'1000'}),'b');
});
test('a new login to the same account, capture or different account resumes following automatically',()=>{
 const manual={profileId:'b',manualSession:'capture:1'};
 assert.equal(selectedProfile(manual,state('a','capture:2')),'a');
 assert.equal(selectedProfile(manual,state('a','new-capture:1')),'a');
 assert.equal(selectedProfile({profileId:'a',manualSession:'capture:1'},state('b','capture:2')),'b');
});
test('offline browsing is allowed; returning to the game and missing profiles have safe defaults',()=>{
 assert.equal(selectedProfile({profileId:'b',manualSession:null},state(null,null)),'b');
 assert.equal(selectedProfile({profileId:'b',manualSession:null},state()),'a');
 assert.equal(selectedProfile({profileId:'deleted',manualSession:'capture:1'},state()),'a');
 assert.equal(selectedProfile({profileId:'deleted',manualSession:null},{profiles:[]}),'');
});
