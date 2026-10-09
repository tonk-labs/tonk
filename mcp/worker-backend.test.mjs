import {test} from 'node:test';
import assert from 'node:assert/strict';
import {workerBackend} from './worker-backend.mjs';
test('general MCP evaluation forwards unchanged notation to the selected worker',async()=>{
  const calls=[];
  const backend=workerBackend({subject:'did:key:z6Test',request:async(path,options)=>{calls.push({path,...options});return Response.json({commits:{claims:1}});}});
  const document='library/install!:\n  component: notebook\n';
  await backend.call('tonk_evaluate',{document});
  await backend.call('tonk_query',{document:'notebook/named:\n'});
  assert.equal(calls[0].path,'/api/repository/did%3Akey%3Az6Test/branch/main/evaluate?transact=true');
  assert.equal(calls[0].body,document);
  assert.ok(calls[1].path.endsWith('transact=false'));
  await assert.rejects(backend.call('tonk_evaluate',{document,subject:'another'}));
  assert.equal(calls.length,2);
});
test('general evaluation never retries an uncertain worker response',async()=>{
  let calls=0;
  const backend=workerBackend({subject:'did:key:z6Test',request:async()=>{calls++;throw Error('connection lost');}});
  await assert.rejects(backend.call('tonk_evaluate',{document:'example!:\n'}));
  assert.equal(calls,1);
});
test('full worker MCP exposes only general operations and keeps browser authority out of model content',async()=>{
  const {createTonkHTTP}=await import('./http.mjs');
  const {Client,StreamableHTTPClientTransport}=await import('@modelcontextprotocol/client');
  const backend={...workerBackend({subject:'did:key:z6Test',request:()=>{throw Error('unexpected');}}),openSpace:async()=>({token:'private-test-token',expiresAt:100})};
  const app=createTonkHTTP(async()=>backend),client=new Client({name:'worker-contract',version:'1'});
  try{
    await client.connect(new StreamableHTTPClientTransport(new URL('http://localhost/mcp'),{fetch:(url,init)=>app.fetch(new Request(url,init))}));
    assert.deepEqual((await client.listTools()).tools.map(t=>t.name).sort(),['tonk_evaluate','tonk_open_space','tonk_query','tonk_space_info']);
    const tool=(await client.listTools()).tools.find(tool=>tool.name==='tonk_open_space');
    const resourceUri=tool._meta.ui.resourceUri;
    const resource=await client.readResource({uri:resourceUri});
    const html=resource.contents[0].text;
    const {createHash}=await import('node:crypto');
    assert.equal(resourceUri,`ui://tonk/worker-space-${createHash('sha256').update(html).digest('hex').slice(0,16)}.html`);
    assert.ok(!html.includes('<header>'));
    const opened=await client.callTool({name:'tonk_open_space',arguments:{path:'/'}});
    assert.equal(opened._meta.session.token,'private-test-token');
    assert.ok(!JSON.stringify([opened.content,opened.structuredContent]).includes('private-test-token'));
  }finally{await client.close();await app.close();}
});

test('account-aware MCP requires explicit targets and passes space to the embedded session',async()=>{
 const {createTonkHTTP}=await import('./http.mjs');
 const {Client,StreamableHTTPClientTransport}=await import('@modelcontextprotocol/client');
 const calls=[];const backend={spaceSelection:true,capabilities:['tonk_guide','tonk_list_spaces','tonk_query','tonk_evaluate','tonk_space_info'],call:async(name,args)=>{calls.push({name,args});return {};},openSpace:async space=>({token:space})};
 const app=createTonkHTTP(async()=>backend),client=new Client({name:'space-contract',version:'1'});
 try{
  await client.connect(new StreamableHTTPClientTransport(new URL('http://localhost/mcp'),{fetch:(url,init)=>app.fetch(new Request(url,init))}));
  const tools=(await client.listTools()).tools;
  assert.ok(!tools.find(t=>t.name==='tonk_guide').inputSchema.properties.space);
  for(const name of ['tonk_query','tonk_evaluate','tonk_space_info','tonk_open_space'])assert.ok(tools.find(t=>t.name===name).inputSchema.required.includes('space'));
  assert.equal((await client.callTool({name:'tonk_query',arguments:{document:'example:'}})).isError,true);assert.equal(calls.length,0);
  await client.callTool({name:'tonk_query',arguments:{document:'example:',space:'did:key:two'}});assert.equal(calls[0].args.space,'did:key:two');
  const opened=await client.callTool({name:'tonk_open_space',arguments:{space:'did:key:one',path:'/'}});
  assert.equal(opened.structuredContent.subject,'did:key:one');assert.equal(opened._meta.session.token,'did:key:one');
 }finally{await client.close();await app.close();}
});
