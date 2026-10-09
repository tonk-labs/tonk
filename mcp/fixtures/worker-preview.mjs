// Full native worker and normal frontend, using only a disposable local space.
import {createServer} from 'node:http';
import {Readable} from 'node:stream';
import {mkdtemp,readFile,rm} from 'node:fs/promises';
import {join} from 'node:path';
import {tmpdir} from 'node:os';
import {startWorker} from '../worker-process.mjs';
import {workerBackend} from '../worker-backend.mjs';
import {createSpaceSessions} from '../space-session.mjs';
import {createInterface} from 'node:readline';
const directory=await mkdtemp(join(tmpdir(),'tonk-full-worker-test-'));
const worker=await startWorker({binary:process.env.TONK_WORKER_BINARY,dataDirectory:directory});
async function json(path,options){const r=await worker.request(path,options);const value=await r.json();if(!r.ok)throw Error(JSON.stringify(value));return value;}
const created=await json('/api/repository/host-test',{method:'PUT',headers:{'content-type':'application/json'},body:JSON.stringify({branch:{main:{}}})});
const subject=created.name.startsWith('did:')?created.name:'did:key:'+created.name;
const backend=workerBackend({subject,request:(path,options)=>worker.request(path,options)});
await backend.call('tonk_evaluate',{document:'library/install!:\n  component: tonk:library/notebook\n  time: 1.0\n'});
await backend.call('tonk_evaluate',{document:'notebook/named!:\n  this: urn:demo:notebook\n  title: Shared Tonk worker\nblock/edit!:\n  subject: urn:demo:block\n  notebook: urn:demo:notebook\n  source: "# Shared Tonk worker\\n\\nEdit this with the normal frontend."\nblock/place!:\n  subject: urn:demo:block\n  notebook: urn:demo:notebook\n  key: N1\n'});
const sessions=createSpaceSessions({resolveWorker:async()=>worker,...(process.env.TONK_TEST_SESSION_LIFETIME_MS?{lifetime:Number(process.env.TONK_TEST_SESSION_LIFETIME_MS)}:{})});
const initial={structuredContent:{subject,path:'/notebook/urn:demo:notebook'},_meta:{session:sessions.issue({fixture:true},subject)}};
const widget=(await readFile(new URL('../ui/worker-space.html',import.meta.url),'utf8')).replaceAll('https://tonk-mcp-test.tonk.workers.dev','http://127.0.0.1:8796').replace("fetch(origin+'/space-api'+path","fetch('http://127.0.0.1:'+(8796+(++rpcId%8))+'/space-api'+path");
const html=`<!doctype html><title>Shared Tonk worker test</title><style>body{margin:0}iframe{width:100%;height:98vh;border:0}</style><iframe title="MCP test host" sandbox="allow-scripts allow-same-origin"></iframe><script>
const frame=document.querySelector('iframe');addEventListener('message',e=>{if(e.source!==frame.contentWindow)return;const m=e.data;if(m.method==='ui/initialize')frame.contentWindow.postMessage({jsonrpc:'2.0',id:m.id,result:{}},'*');if(m.method==='ui/notifications/initialized')frame.contentWindow.postMessage({jsonrpc:'2.0',method:'ui/notifications/tool-result',params:${JSON.stringify(initial)}},'*');});frame.srcdoc=${JSON.stringify(widget).replace(/</g,'\\u003c')};</script>`;
const handler=async(req,res)=>{try{
 if(!/^127\.0\.0\.1:8(?:79[6-9]|80[0-3])$/.test(req.headers.host)){res.writeHead(403).end();return;}
 const url=new URL(req.url,'http://127.0.0.1:8796');
 if(url.pathname==='/'){res.writeHead(200,{'content-type':'text/html'}).end(html);return;}
 if(['/worker-guest.html','/space-runtime.json'].includes(url.pathname)){res.writeHead(200,{'content-type':url.pathname.endsWith('.json')?'application/json':'text/html'}).end(await readFile(new URL('../public'+url.pathname,import.meta.url)));return;}
 if(url.pathname.startsWith('/space-api/')){
  const abort=new AbortController();res.on('close',()=>abort.abort());
  const chunks=[];for await(const chunk of req)chunks.push(chunk);
  const response=await sessions.fetch(new Request(url,{method:req.method,headers:req.headers,signal:abort.signal,...(['GET','HEAD'].includes(req.method)?{}:{body:Buffer.concat(chunks)})}));
  if(url.pathname==='/space-api/session/renew')console.log('Fixture session renewal HTTP '+response.status);
  res.writeHead(response.status,Object.fromEntries(response.headers));if(response.body)Readable.fromWeb(response.body).on('error',()=>res.destroy()).pipe(res);else res.end();return;
 }
 res.writeHead(404).end();
}catch(error){console.error(error.message);if(!res.headersSent)res.writeHead(500);res.end();}};
// Browsers cap HTTP/1 at six SSE streams per origin. Production uses HTTP/2;
// the local fixture spreads streams over eight loopback ports to test real SSE.
const servers=Array.from({length:8},(_,i)=>createServer(handler).listen(8796+i,'127.0.0.1'));
console.log('Shared Tonk worker test: http://127.0.0.1:8796');
process.on('SIGINT',async()=>{sessions.close();for(const server of servers){server.close();server.closeAllConnections();}await worker.close();await rm(directory,{recursive:true,force:true});process.exit();});

createInterface({input:process.stdin}).on('line',async line=>{if(line==='agent'){await backend.call('tonk_evaluate',{document:'block/edit!:\n  subject: urn:demo:block\n  notebook: urn:demo:notebook\n  source: Updated by the general MCP evaluator.\n'});console.log('General evaluator write complete');}if(line==='read')console.log(JSON.stringify(await backend.call('tonk_query',{document:'notebook/block:\n  notebook: urn:demo:notebook\n  source: ?source\n'})));});
