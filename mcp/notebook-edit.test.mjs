import {test} from 'node:test';
import assert from 'node:assert/strict';
import {editNotebook} from './notebook-edit.mjs';
const entity='urn:demo:notebook',block='urn:demo:block';
const request={claims:[{op:'assert',application:{predicate:{kind:'transient',concept:{with:{subject:{the:'xyz.tonk.block.edit/subject'},notebook:{the:'xyz.tonk.block.edit/notebook'},source:{the:'xyz.tonk.block.edit/source'}}}},parameters:{subject:block,notebook:entity,source:'Updated text'}}}]};
function fixture(){const calls=[];return {calls,async call(name,args){calls.push({name,args});if(name==='tonk_query')return {revision:{tree:'before'},matches:[{results:[{this:entity,fields:{title:'Test'}}]},{results:[{this:entity,fields:{block:{N1:block}}}]},{results:[{this:block,fields:{notebook:entity,source:'Original'}}]}]};if(name==='tonk_space_info')return {subject:'did:key:z6Test'};if(name==='tonk_preview')return {revision:{tree:'before'}};if(name==='tonk_apply')return {accepted:true,revision:{tree:'after'}};throw Error(name);}};}
test('notebook text uses one canonical conditional apply',async()=>{const backend=fixture();const result=await editNotebook(backend,{entity,request,expectedRevision:{tree:'before'}});assert.equal(result.accepted,true);assert.deepEqual(backend.calls.map(c=>c.name),['tonk_query','tonk_space_info','tonk_preview','tonk_apply']);assert.match(backend.calls.at(-1).args.document,/source: "Updated text"/);});
test('stale revision and cross-notebook block never reach apply',async()=>{for(const args of [{entity,request,expectedRevision:{tree:'old'}},{entity,request:{claims:[{...request.claims[0],application:{...request.claims[0].application,parameters:{...request.claims[0].application.parameters,subject:'urn:other'}}}]},expectedRevision:{tree:'before'}}]){const backend=fixture();await assert.rejects(editNotebook(backend,args));assert.ok(!backend.calls.some(c=>c.name==='tonk_apply'));}});
test('an uncertain write is not retried',async()=>{const backend=fixture(),call=backend.call.bind(backend);backend.call=async(name,args)=>{const result=await call(name,args);if(name==='tonk_apply')throw Error('push uncertain');return result;};await assert.rejects(editNotebook(backend,{entity,request,expectedRevision:{tree:'before'}}));assert.equal(backend.calls.filter(c=>c.name==='tonk_apply').length,1);});

test('real native edit persists through reopening and rejects a stale retry',{skip:!process.env.TONK_MCP_RUNTIME},async()=>{
  const {startNativeRuntime}=await import('./native.mjs');
  const {mkdtemp,rm}=await import('node:fs/promises');
  const {tmpdir}=await import('node:os');
  const {readNotebook}=await import('./notebook.mjs');
  const directory=await mkdtemp(tmpdir()+'/tonk-edit-test-');
  let runtime;
  try{
    runtime=await startNativeRuntime({binary:process.env.TONK_MCP_RUNTIME,dataDirectory:directory});
    const install=await runtime.call('tonk_install_library',{component:'notebook'});
    await runtime.call('tonk_install_library',{component:'notebook',expectedRevision:install.revision});
    const document=`notebook/named!:\n  this: ${entity}\n  title: Test\nnotebook/block!:\n  this: ${block}\n  notebook: ${entity}\n  source: Original\nnotebook/blocks!:\n  this: ${entity}\n  block: {N1: ${block}}\n`;
    const preview=await runtime.call('tonk_preview',{document});await runtime.call('tonk_apply',{document,expectedRevision:preview.revision});
    const before=await readNotebook(runtime,entity);
    const edit=structuredClone(request);edit.claims[0].application.parameters.source='Updated text!';
    await editNotebook(runtime,{entity,request:edit,expectedRevision:before.revision});
    await assert.rejects(editNotebook(runtime,{entity,request:edit,expectedRevision:before.revision}),/changed/);
    await runtime.close();runtime=await startNativeRuntime({binary:process.env.TONK_MCP_RUNTIME,dataDirectory:directory});
    assert.equal((await readNotebook(runtime,entity)).markdown,'Updated text!');
  }finally{await runtime?.close();await rm(directory,{recursive:true,force:true});}
});
