// Real Tonk guest renderer + real native replica, with a synthetic local host.
import {createServer} from 'node:http';
import {mkdtemp,readFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {startNativeRuntime} from '../native.mjs';
import {readNotebook} from '../notebook.mjs';
import {editNotebook} from '../notebook-edit.mjs';
import {spaceHTML} from '../space-ui.mjs';
const directory=await mkdtemp(join(tmpdir(),'tonk-space-ui-'));
const runtime=await startNativeRuntime({binary:process.env.TONK_MCP_RUNTIME,dataDirectory:directory});
const library=await runtime.call('tonk_install_library',{component:'notebook'});
await runtime.call('tonk_install_library',{component:'notebook',expectedRevision:library.revision});
const doc='notebook/named!:\n  this: urn:demo:notebook\n  title: "Actual Tonk frontend"\nnotebook/block!:\n  this: urn:demo:block\n  notebook: urn:demo:notebook\n  source: "# Real notebook\\n\\nThis is rendered by the actual **Tonk notebook component**."\nnotebook/blocks!:\n  this: urn:demo:notebook\n  block: {N1: urn:demo:block}\n';
const preview=await runtime.call('tonk_preview',{document:doc});await runtime.call('tonk_apply',{document:doc,expectedRevision:preview.revision});
const initial=await runtime.uiRead({path:'/notebook/urn:demo:notebook'});
const rootRoute=await runtime.uiRead({path:'/'});
const widget=spaceHTML.replaceAll('https://tonk-mcp-test.tonk.workers.dev/',(process.env.TONK_SPACE_ASSET_ORIGIN||'http://127.0.0.1:8795')+'/');
const html=`<!doctype html><meta charset="utf-8"><title>Real Tonk space test</title><style>body{margin:0}iframe{width:100%;height:95vh;border:0}</style><iframe sandbox="allow-scripts allow-same-origin" title="MCP host test"></iframe><script>
const frame=document.querySelector('iframe');const send=m=>frame.contentWindow.postMessage({jsonrpc:'2.0',...m},'*');
addEventListener('message',async e=>{if(e.source!==frame.contentWindow)return;const m=e.data;if(m.method==='ui/initialize')send({id:m.id,result:{protocolVersion:'2026-01-26'}});else if(m.method==='ui/notifications/initialized')send({method:'ui/notifications/tool-result',params:{structuredContent:${JSON.stringify(initial)}}});else if(m.method==='tools/call'){try{const r=await fetch('/tool',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(m.params)});send({id:m.id,result:await r.json()});}catch{send({id:m.id,error:{message:'Local test failed'}});}}});
frame.srcdoc=${JSON.stringify(widget).replace(/</g,'\\u003c')};</script>`;
const server=createServer(async(req,res)=>{try{if(req.headers.host!=='127.0.0.1:8795'){res.writeHead(403).end();return;}
if(req.url==='/'||req.url==='/?root'){res.writeHead(200,{'content-type':'text/html'}).end(req.url==='/?root'?html.replace(JSON.stringify(initial),JSON.stringify(rootRoute)):html);return;}
if(req.url.split('?')[0]==='/space-guest.html'){res.writeHead(200,{'content-type':'text/html'}).end(await readFile(new URL('../public/space-guest.html',import.meta.url)));return;}
if(req.url==='/space-runtime.json'){res.writeHead(200,{'content-type':'application/json'}).end(await readFile(new URL('../public/space-runtime.json',import.meta.url)));return;}
if(req.url==='/tool'&&req.method==='POST'){let body='';for await(const chunk of req){body+=chunk;if(body.length>200000)throw Error('Too large');}const m=JSON.parse(body);let result;if(m.name==='tonk_open_space')result={structuredContent:await runtime.uiRead({path:m.arguments.path})};else if(m.name==='tonk_ui_begin_edit')result={_meta:{notebook:await readNotebook(runtime,m.arguments.entity)}};else if(m.name==='tonk_ui_edit_notebook')result={_meta:{saved:await editNotebook(runtime,m.arguments)}};else if(m.name==='tonk_ui_read'){const rows=[];for(const query of m.arguments.queries)rows.push((await runtime.uiRead({path:m.arguments.path,query})).rows);result={_meta:{rows}};}else throw Error('Unexpected tool');res.writeHead(200,{'content-type':'application/json'}).end(JSON.stringify(result));return;}
res.writeHead(404).end();}catch(e){console.error(e.message);res.writeHead(200,{'content-type':'application/json'}).end(JSON.stringify({isError:true,content:[{type:'text',text:e.message}]}));}});
server.listen(8795,'127.0.0.1',()=>console.log('Real Tonk frontend test: http://127.0.0.1:8795'));
process.on('SIGINT',async()=>{server.close();await runtime.close();await rm(directory,{recursive:true,force:true});process.exit();});
