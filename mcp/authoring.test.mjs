import {test} from 'node:test';import assert from 'node:assert/strict';
import {readFile,mkdtemp,rm} from 'node:fs/promises';import {tmpdir} from 'node:os';import {join} from 'node:path';
import {authoringGuide} from './authoring.mjs';import {startWorker} from './worker-process.mjs';import {workerBackend} from './worker-backend.mjs';
test('bundled authoring manuals match canonical notation, views and events',async()=>{
 for(const [topic,path] of [['notation','../rust/tonk-notation/guide.md'],['views','../rust/tonk-cli/src/guide-views.md'],['events','../rust/tonk-cli/src/guide-events.md']])assert.equal(authoringGuide({topic}).text,await readFile(new URL(path,import.meta.url),'utf8'));
 assert.ok(authoringGuide().topics.includes('notation'));assert.throws(()=>authoringGuide({topic:'../../private'}));
});
test('documented native schema and views create three tasks and query them back',{skip:!process.env.TONK_WORKER_BINARY},async()=>{
 const directory=await mkdtemp(join(tmpdir(),'tonk-authoring-'));const worker=await startWorker({binary:process.env.TONK_WORKER_BINARY,dataDirectory:directory});
 try{
  const created=await (await worker.request('/api/repository/authoring-test',{method:'PUT',headers:{'content-type':'application/json'},body:JSON.stringify({branch:{main:{}}})})).json();
  const subject=created.name.startsWith('did:')?created.name:'did:key:'+created.name;
  const backend=workerBackend({subject,request:(path,options)=>worker.request(path,options)});
  await backend.call('tonk_evaluate',{document:await readFile(new URL('./fixtures/authoring/tasks.notation',import.meta.url),'utf8')});
  const guide=authoringGuide({topic:'views'}).text;
  const example=name=>{
    const marker='<!-- tested-example: '+name+' -->';
    const block=guide.split(marker)[1]?.match(/```yaml tonk=parse\n([\s\S]*?)```/);
    assert.ok(block,'missing executable guide example '+name);return block[1];
  };
  await backend.call('tonk_evaluate',{document:example('task-views')+'\n'+example('space-home')});
  const home=await backend.call('tonk_query',{document:'name:\n  this: id:tonk/space\n'});
  assert.equal(home.matches_after[0].results[0].fields.entity,'space:home');
  const homeView=await backend.call('tonk_query',{document:'xyz.tonk.view:\n  this: space:home\n  ui: _\n'});
  assert.match(homeView.matches_after[0].results[0].fields.ui,/model=integration-task/);
  const views=await backend.call('tonk_query',{document:'xyz.tonk.view:\n  this: integration-task\n  ui: _\n  directory: _\n'});
  const fields=views.matches_after[0].results[0].fields;
  assert.match(fields.ui,/checked=\{done\}/);
  assert.match(fields.directory,/model=integration-task view=ui/);
  const result=await backend.call('tonk_query',{document:'integration-task:\n  title: ?title\n  done: ?done\n'});
  const matches=result.matches_after??result.matches_before;
  assert.equal(matches[0].results.length,3);
  const site=await worker.request('/api/repository/'+encodeURIComponent(subject)+'/branch/main/site',{method:'POST',headers:{'content-type':'application/json','x-tonk-client-id':'authoring-test'},body:JSON.stringify({path:'/integration-task'})});
  assert.equal(site.status,200,await site.text());
  const root=await worker.request('/api/repository/'+encodeURIComponent(subject)+'/branch/main/site',{method:'POST',headers:{'content-type':'application/json','x-tonk-client-id':'authoring-test'},body:JSON.stringify({path:'/'})});
  assert.equal(root.status,200,await root.text());
  const values=matches[0].results.map(row=>row.fields.done);
  assert.deepEqual(values.sort(),[false,false,true]);
 }finally{await worker.close();await rm(directory,{recursive:true,force:true});}
});
