import {test} from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {workerAccountProvisioner} from './worker-tenants.mjs';

test('worker account activation identifies failed stage without exposing responses', async()=>{
  const directory=await mkdtemp(join(tmpdir(),'tonk-activation-test-'));
  try {
    for(const failure of ['root-import','account-attach','account-hydration',null]) {
      let closed=0;
      const provision=workerAccountProvisioner({dataRoot:directory,start:async()=>({
        close:async()=>{closed++;},request:async path=>{
          if(path==='/api/identify')return Response.json({did:'did:key:device'});
          if(path==='/api/identity/root')return failure==='root-import'?new Response('private grant',{status:400}):Response.json({status:'ready',rootDid:'did:key:root',deviceDid:'did:key:device'});
          if(failure==='account-attach')return new Response('private upstream error',{status:403});
          return Response.json({status:'registered',rootDid:'did:key:root',deviceDid:'did:key:device',accountState:failure==='account-hydration'?'unhydrated':'ready'});
        },
      })});
      const account=await provision();
      const activated=account.authorize({remote:'https://tonk.foundation/ucan/',credentialId:'credential',delegationHex:'private'});
      if(failure)await assert.rejects(activated,error=>error.authorizationStage===failure&&!error.message.includes('private'));
      else assert.deepEqual(await activated,{rootDid:'did:key:root',deviceDid:'did:key:device'});
      assert.equal(closed,2);
    }
  } finally {await rm(directory,{recursive:true,force:true});}
});

test('account catalog selects multiple spaces without changing tenant binding or push targets',async()=>{
 const {createWorkerTenants}=await import('./worker-tenants.mjs');
 const {mkdir}=await import('node:fs/promises');
 const directory=await mkdtemp(join(tmpdir(),'tonk-selection-test-'));
 await mkdir(join(directory,'tenant-abcdef'));
 const calls=[];let starts=0;
 const tenants=createWorkerTenants({dataRoot:directory,start:async()=>{starts++;return {close:async()=>{},request:async(path,options={})=>{
  calls.push(path);
  if(path==='/api/identity/root')return Response.json({rootDid:'root',deviceDid:'device'});
  if(path.includes('/profile/branch/meta/query')){
   const body=JSON.parse(options.body);return Response.json(body.terms.branch?[{fields:{branch:'urn:branch'}}]:[{fields:{name:'active'}}]);
  }
  if(path==='/api/profile/branch/active/query')return Response.json(['did:key:one','did:key:two'].map(subject=>({fields:{subject,name:'Same name'}})));
  return Response.json({});
 }}}});
 const principal={tenantId:'tenant-abcdef',rootDid:'root',deviceDid:'device'};
 try{
  assert.deepEqual((await tenants.list(principal)).map(s=>s.subject),['did:key:one','did:key:two']);
  const one=await tenants.resolve(principal,'did:key:one'),two=await tenants.resolve(principal,'did:key:two');
  assert.equal(one,two);assert.equal(starts,1);
  await assert.rejects(tenants.resolve(principal,'did:key:unknown'),/not in/);
  assert.ok(!calls.includes('/api/repository/did%3Akey%3Aunknown'));
  await assert.rejects(tenants.resolve({...principal,rootDid:'other'},'did:key:one'),/binding/);
  await two.request('/api/repository/did%3Akey%3Atwo/branch/main/evaluate?transact=true',{method:'POST'});
  assert.equal(calls.at(-1),'/api/repository/did%3Akey%3Atwo/branch/main/sync/push');
 }finally{await tenants.close();await rm(directory,{recursive:true,force:true});}
});

test('dead account worker restarts on the next call using its live directory, without replaying a write',async()=>{
 const {createWorkerTenants}=await import('./worker-tenants.mjs');const {mkdir}=await import('node:fs/promises');
 const directory=await mkdtemp(join(tmpdir(),'tonk-worker-recovery-'));await mkdir(join(directory,'tenant-abcdef'));
 const directories=[];let alive=true,writes=0,starts=0;
 const tenants=createWorkerTenants({dataRoot:directory,start:async({dataDirectory})=>{
  directories.push(dataDirectory);starts++;alive=true;
  return {isRunning:()=>alive,close:async()=>{},request:async(path)=>{
   if(path==='/api/identity/root')return Response.json({rootDid:'root',deviceDid:'device'});
   if(path.endsWith('/evaluate?transact=true')){writes++;alive=false;throw Error('private transport details');}
   return Response.json({saved:'previous local changes'});
  }};
 }});
 const principal={tenantId:'tenant-abcdef',rootDid:'root',deviceDid:'device'};
 try{
  const worker=await tenants.account(principal);
  await assert.rejects(worker.request('/api/repository/did%3Akey%3Aone/branch/main/evaluate?transact=true',{method:'POST'}),error=>/interrupted/.test(error.message)&&!error.message.includes('private'));
  assert.equal(starts,1);assert.equal(writes,1);
  await Promise.all([worker.request('/read'),worker.request('/read')]);
  assert.equal(starts,2);assert.equal(writes,1);assert.equal(directories[0],directories[1]);
  assert.equal(await tenants.account(principal),worker);
 }finally{await tenants.close();await rm(directory,{recursive:true,force:true});}
});
