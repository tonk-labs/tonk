import {test} from 'node:test';
import assert from 'node:assert/strict';
import {Client,StreamableHTTPClientTransport} from '@modelcontextprotocol/client';
import {spaceURI} from './space-ui.mjs';
import {createTonkHTTP} from './http.mjs';
test('space bridge exposes app-only reads without publishing private rows to model content',async()=>{
  const calls=[];
  const app=createTonkHTTP(async()=>({capabilities:[],call(){throw Error('Unexpected data tool');},async uiRead(args){calls.push(args);return {subject:'did:key:z6Test',path:args.path,model:'id:notebook/route',site:'site:chatgpt',rows:[{this:'urn:private',fields:{title:'Private title'}}]};}}));
  const client=new Client({name:'space-test',version:'1'});
  try{
    await client.connect(new StreamableHTTPClientTransport(new URL('http://localhost/mcp'),{fetch:(url,init)=>app.fetch(new Request(url,init))}));
    const tools=(await client.listTools()).tools;
    assert.deepEqual(tools.find(t=>t.name==='tonk_ui_read')._meta.ui.visibility,['app']);
    const result=await client.callTool({name:'tonk_ui_read',arguments:{path:'/',queries:[{predicate:{},terms:{}}]}});
    assert.deepEqual(result.content,[]);assert.equal(result.structuredContent,undefined);assert.equal(result._meta.rows[0][0].fields.title,'Private title');
    const before=calls.length;
    const invalid=await client.callTool({name:'tonk_ui_read',arguments:{path:'/',queries:[{}],repository:'other'}});
    assert.equal(invalid.isError,true);assert.equal(calls.length,before);
    const resource=await client.readResource({uri:spaceURI});
    assert.match(resource.contents[0].text,/sandbox="allow-scripts allow-forms"/);
  }finally{await client.close();await app.close();}
});

test('renderer batches stop at a bounded private response size',async()=>{
  let calls=0;
  const app=createTonkHTTP(async()=>({capabilities:[],async uiRead(){calls++;return {rows:[{value:'x'.repeat(600_000)}]};}}));
  const client=new Client({name:'space-limit-test',version:'1'});
  try {
    await client.connect(new StreamableHTTPClientTransport(new URL('http://localhost/mcp'),{fetch:(url,init)=>app.fetch(new Request(url,init))}));
    const result=await client.callTool({name:'tonk_ui_read',arguments:{path:'/',queries:[{},{},{}]}});
    assert.equal(result.isError,true);
    assert.equal(result._meta,undefined);
    assert.equal(calls,2);
  } finally {await client.close();await app.close();}
});

 test('editing tools are app-only writes and reject extra authority arguments', async()=>{
  let calls=0;
  const app=createTonkHTTP(async()=>({capabilities:['tonk_apply'],uiRead(){},call(){calls++;throw Error('Unexpected call');}}));
  const client=new Client({name:'edit-test',version:'1'});
  try {
    await client.connect(new StreamableHTTPClientTransport(new URL('http://localhost/mcp'),{fetch:(url,init)=>app.fetch(new Request(url,init))}));
    const tools=(await client.listTools()).tools;
    const edit=tools.find(t=>t.name==='tonk_ui_edit_notebook');
    assert.deepEqual(edit._meta.ui.visibility,['app']);
    assert.equal(edit.annotations.readOnlyHint,false);
    assert.equal(edit.annotations.idempotentHint,false);
    const rejected=await client.callTool({name:edit.name,arguments:{entity:'urn:demo:notebook',request:{claims:[]},expectedRevision:null,repository:'another'}});
    assert.equal(rejected.isError,true); assert.equal(calls,0);
  }finally{await client.close();await app.close();}
});
