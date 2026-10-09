import {test} from 'node:test';import assert from 'node:assert/strict';import {accountSpaces} from './account-spaces.mjs';
test('profile route compatibility falls back only on absent routes',async()=>{
 for(const status of [200,404,405,500]){
  const paths=[];const operation=accountSpaces({request:async path=>{paths.push(path);if(paths.length===1)return new Response(null,{status});return Response.json(path.includes('/meta/')?[]:[{fields:{subject:'did:key:space',name:'Example'}}]);}});
  if(status===500){await assert.rejects(operation,/unavailable/);assert.equal(paths.length,1);}
  else{assert.equal((await operation)[0].subject,'did:key:space');assert.equal(paths[1],(status===200?'/api/profile/branch/':'/api/repository/profile:tonk/branch/')+'meta/query');}
 }
});
