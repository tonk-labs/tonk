// Full-worker hosted runtime. OAuth seeds are checkpointed while closed;
// long-lived worker replicas publish through Tonk's normal remote sync.
import {createServer} from 'node:http';
import {Readable} from 'node:stream';
import {mkdtemp} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {createDurableState} from './durable-state.mjs';
import {createCheckpoint} from './checkpoint.mjs';
import {createTonkOAuth} from './oauth.mjs';
import {createAuthenticatedTonkHTTP} from './authenticated-http.mjs';
import {workerAccountProvisioner,createWorkerTenants} from './worker-tenants.mjs';
import {authoringGuide} from './authoring.mjs';
import {workerBackend} from './worker-backend.mjs';
import {createSpaceSessions} from './space-session.mjs';
const required=name=>{if(!process.env[name])throw Error('Missing '+name);return process.env[name];};
const issuer=required('TONK_MCP_ISSUER'),subject=required('TONK_MCP_SPACE'),binary=required('TONK_WORKER_BINARY');
if(new URL(issuer).origin!==issuer||!issuer.startsWith('https://'))throw Error('Invalid issuer');
const dataRoot=await mkdtemp(join(tmpdir(),'tonk-worker-seeds-'));
const checkpoint=await createCheckpoint({directory:dataRoot});
const durable=await createDurableState();
if(durable.state&&(durable.state.version!==1||durable.state.subject!==subject))throw Error('Credential snapshot configuration mismatch');
let provisioning=Promise.resolve();
const provision=workerAccountProvisioner({binary,dataRoot});
const oauth=createTonkOAuth({issuer,clientId:required('TONK_MCP_CLIENT_ID'),redirectUris:[required('TONK_MCP_REDIRECT_URI')],linkPage:'https://tonk.foundation/settings/link',
  saved:durable.state?.oauth,
  onRevoke:principal=>sessions.revokePrincipal(principal),
  provisionAccount:async()=>{
    const opening=provisioning.then(()=>provision());
    provisioning=opening.catch(()=>{});
    const account=await opening;
    return {...account,authorize(authorization){
      const result=provisioning.then(async()=>{const identity=await account.authorize(authorization);try { await checkpoint.save({roots:new Set([account.tenantId,...oauth.retainedTenants(),...sessions.retainedTenants()])}); } catch (cause) {
        console.error(JSON.stringify({event:'authorization-checkpoint-failed',reason:['file-limit','expanded-limit','compressed-limit','storage-rejected'].includes(cause?.checkpointReason)?cause.checkpointReason:'unavailable',status:Number.isInteger(cause?.checkpointStatus)?cause.checkpointStatus:undefined})); throw Object.assign(Error('Checkpoint failed'), {authorizationStage:'checkpoint'}); } return identity;});
      provisioning=result.catch(()=>{});return result;
    }};
  },
});
const tenants=createWorkerTenants({binary,dataRoot});
const sessions=createSpaceSessions({saved:durable.state?.sessions,resolveWorker:(principal,selected)=>tenants.resolve(principal,selected)});
const http=createAuthenticatedTonkHTTP({oauth,resolveBackend:async principal=>{
  return {
    spaceSelection:true,capabilities:['tonk_guide','tonk_list_spaces','tonk_query','tonk_evaluate','tonk_space_info'],
    async call(name,args,signal){
      if(name==='tonk_guide')return authoringGuide(args);
      if(name==='tonk_list_spaces')return {spaces:await tenants.list(principal)};
      const {space,...input}=args;
      const worker=await tenants.resolve(principal,space);
      return workerBackend({subject:space,request:(path,options)=>worker.request(path,options)}).call(name,input,signal);
    },
    async openSpace(space){await tenants.resolve(principal,space);return sessions.issue(principal,space);},
  };
}});
const server=createServer(async(req,res)=>{
  try{
    if(!req.url?.startsWith('/')||req.url.startsWith('//')){res.writeHead(400).end();return;}
    const abort=new AbortController();res.on('close',()=>abort.abort());
    let size=0;const chunks=[];
    for await(const chunk of req){size+=chunk.length;if(size>200000){res.writeHead(413).end();return;}chunks.push(chunk);}
    const request=new Request(issuer+req.url,{method:req.method,headers:req.headers,signal:abort.signal,...(['GET','HEAD'].includes(req.method)?{}:{body:Buffer.concat(chunks)})});
    const response=await durable.run(()=>new URL(request.url).pathname.startsWith('/space-api/')?sessions.fetch(request):http.fetch(request),()=>({version:1,subject,oauth:oauth.snapshot(),sessions:sessions.snapshot()}));
    res.writeHead(response.status,Object.fromEntries(response.headers));
    if(response.body)Readable.fromWeb(response.body).on('error',()=>res.destroy()).pipe(res);else res.end();
  }catch{if(!res.headersSent)res.writeHead(503);res.end();}
});
server.listen(8080,'0.0.0.0');
async function close(){server.close();sessions.close();await http.close();await tenants.close();server.closeAllConnections();}
process.once('SIGTERM',close);process.once('SIGINT',close);
