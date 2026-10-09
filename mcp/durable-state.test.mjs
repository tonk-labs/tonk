import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createDurableState} from './durable-state.mjs';
import {createCredentials} from './credentials.mjs';

test('acknowledged rotation survives restart and replay revocation persists',async()=>{
 let body,revision=0;
 const request=async(url,options={})=>{
  if(options.method!=='PUT')return body?new Response(body,{headers:{etag:String(revision)}}):new Response(null,{status:404});
  assert.equal(options.headers['if-match']??options.headers['if-none-match'],revision?String(revision):'*');
  body=options.body;return new Response(null,{headers:{etag:String(++revision)}});
 };
 const first=await createDurableState({fetch:request});let credentials=createCredentials();
 const original=await first.run(()=>credentials.issue({tenantId:'tenant'}),()=>credentials.snapshot());
 const second=await createDurableState({fetch:request});credentials=createCredentials({saved:second.state});
 assert.equal(credentials.get(original.token).tenantId,'tenant');
 const rotated=await second.run(()=>credentials.renew(original.refreshToken),()=>credentials.snapshot());
 const third=await createDurableState({fetch:request});credentials=createCredentials({saved:third.state});
 assert.equal(credentials.get(rotated.token).tenantId,'tenant');
 await third.run(()=>credentials.renew(original.refreshToken),()=>credentials.snapshot());
 const fourth=await createDurableState({fetch:request});credentials=createCredentials({saved:fourth.state});
 assert.equal(credentials.get(rotated.token),undefined);
 assert.ok(!body.includes(original.token)&&!body.includes(rotated.refreshToken));
});
test('ambiguous persistence failure returns no credentials and fences subsequent requests',async()=>{
 let calls=0;
 const state=await createDurableState({fetch:async(url,options={})=>{
  if(!options.method)return new Response(null,{status:404});calls++;throw Error('lost response');
 }});
 await assert.rejects(state.run(()=>({token:'must not escape'}),()=>({changed:true})),/lost response/);
 let executed=false;await assert.rejects(state.run(()=>{executed=true;},()=>({})),/unavailable/);
 assert.equal(executed,false);assert.equal(calls,1);
});
test('persistence serializes concurrent mutations before acknowledging each',async()=>{
 const writes=[];let value=0;
 const state=await createDurableState({fetch:async(url,options={})=>{
  if(!options.method)return new Response(null,{status:404});writes.push(JSON.parse(options.body));return new Response(null,{headers:{etag:String(writes.length)}});
 }});
 assert.deepEqual(await Promise.all([state.run(()=>++value,()=>value),state.run(()=>++value,()=>value)]),[1,2]);
 assert.deepEqual(writes,[1,2]);
});
