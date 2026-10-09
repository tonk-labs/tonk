import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createSpaceSessions} from './space-session.mjs';
test('browser session is space scoped, expires, and streams directly from the worker',async()=>{
  let now=0,calls=0,controller;
  const principal={tenantId:'test'};
  const sessions=createSpaceSessions({now:()=>now,lifetime:100,resolveWorker:async(p,subject)=>{
    assert.equal(p,principal);assert.equal(subject,'did:key:z6Test');
    return {request:async(path,options)=>{calls++;assert.ok(path.startsWith('/api/repository/did%3Akey%3Az6Test/'));assert.equal(options.headers.has('authorization'),false);return new Response(new ReadableStream({start(c){controller=c;}}),{headers:{'content-type':'text/event-stream'}});}};
  }});
  const {token}=sessions.issue(principal,'did:key:z6Test');
  const request=(subject='did:key:z6Test',action='query')=>new Request('https://host/space-api/api/repository/'+encodeURIComponent(subject)+'/branch/main/'+action,{method:'POST',headers:{authorization:'Bearer '+token},body:'{}'});
  assert.equal((await sessions.fetch(request('did:key:z6Other'))).status,403);
  assert.equal((await sessions.fetch(request('did:key:z6Test','../profile'))).status,403);
  assert.equal(calls,0);
  const response=await sessions.fetch(request());
  const reader=response.body.getReader();controller.enqueue(new TextEncoder().encode('data: snapshot\n\n'));
  assert.equal(new TextDecoder().decode((await reader.read()).value),'data: snapshot\n\n');
  controller.close();await reader.cancel();
  now=100;assert.equal((await sessions.fetch(request())).status,401);assert.equal(calls,1);
});

test('space renewal preserves client context and streams; replay aborts the connection',async()=>{
 let now=0,calls=0;const contexts=[],signals=[];
 const sessions=createSpaceSessions({now:()=>now,lifetime:100,maxLifetime:1000,resolveWorker:async()=>({request:async(path,options)=>{
  calls++;contexts.push(options.headers.get('x-tonk-client-id'));signals.push(options.signal);return new Response('ok');
 }})});
 const first=sessions.issue({tenantId:'one'},'did:key:z6Test');
 const query=token=>sessions.fetch(new Request('https://host/space-api/api/repository/did%3Akey%3Az6Test/branch/main/query',{headers:{authorization:'Bearer '+token}}));
 const renew=token=>sessions.fetch(new Request('https://host/space-api/session/renew',{method:'POST',headers:{authorization:'Bearer '+token}}));
 assert.equal((await query(first.token)).status,200);now=101;
 const response=await renew(first.refreshToken);assert.equal(response.status,200);const second=await response.json();
 assert.equal(calls,1);assert.equal(signals[0].aborted,false);
 assert.equal((await query(first.token)).status,401);assert.equal((await query(second.token)).status,200);assert.equal(contexts[0],contexts[1]);
 assert.equal((await renew(first.refreshToken)).status,401);assert.equal(signals[0].aborted,true);assert.equal((await query(second.token)).status,401);
 sessions.close();
});

test('cached widget grant reopens after restart without replaying a spent refresh credential',async()=>{
 let now=0,calls=0;
 const resolveWorker=async()=>({request:async()=>{calls++;return new Response('ok');}});
 const original=createSpaceSessions({resolveWorker,now:()=>now,lifetime:100,maxLifetime:1000});
 const principal={tenantId:'tenant',deviceDid:'device',connectionExpiresAt:1000};
 const subject='did:key:z6MkExample';
 const initial=original.issue(principal,subject);
 const request=(action,token)=>new Request('https://example.test/space-api/session/'+action,{method:'POST',headers:{authorization:'Bearer '+token}});
 const rotated=await (await original.fetch(request('renew',initial.refreshToken))).json();
 const saved=JSON.parse(JSON.stringify(original.snapshot()));original.close();now=150;
 const restored=createSpaceSessions({resolveWorker,now:()=>now,lifetime:100,maxLifetime:1000,saved});
 const resumed=await (await restored.fetch(request('resume',initial.resumeToken))).json();
 assert.ok(resumed.token);assert.equal(resumed.refreshExpiresAt,1000);assert.equal(calls,0);
 const read=await restored.fetch(new Request('https://example.test/space-api/api/repository/'+subject+'/branch/main/query',{headers:{authorization:'Bearer '+resumed.token}}));
 assert.equal(read.status,200);assert.equal(calls,1);
 assert.ok((await restored.fetch(request('renew',rotated.refreshToken))).ok);
 restored.revokePrincipal(principal);
 assert.equal((await restored.fetch(request('resume',initial.resumeToken))).status,401);
 const revoked=createSpaceSessions({resolveWorker,now:()=>now,saved:restored.snapshot()});
 assert.equal((await revoked.fetch(request('resume',initial.resumeToken))).status,401);
 now=1000;const expired=createSpaceSessions({resolveWorker,now:()=>now,saved});
 assert.equal((await expired.fetch(request('resume',initial.resumeToken))).status,401);
});

test('recovery retains widget resume authority until its fixed expiry',()=>{
 let now=0;const sessions=createSpaceSessions({now:()=>now,lifetime:10,maxLifetime:100,resolveWorker:async()=>{throw Error('not used');}});
 sessions.issue({tenantId:'active'},'did:key:z6Test');
 now=11;assert.deepEqual([...sessions.retainedTenants()],['active']);
 const restored=createSpaceSessions({now:()=>now,lifetime:10,maxLifetime:100,saved:sessions.snapshot()});
 assert.deepEqual([...restored.retainedTenants()],['active']);
 now=101;assert.deepEqual([...restored.retainedTenants()],[]);
});
