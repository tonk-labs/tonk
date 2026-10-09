import {createCredentials} from './credentials.mjs';
import {randomBytes,createHash} from 'node:crypto';

// Short-lived, space-only browser authority. The OAuth token stays in the MCP
// host. Renderer requests use HTTP directly, not tools/call or its write queue.
export function createSpaceSessions({resolveWorker, now=Date.now, lifetime=10*60_000, maxLifetime=8*60*60_000, capacity=64,saved}) {
  const hash=token=>typeof token==='string'&&/^[A-Za-z0-9_-]{43}$/.test(token)?createHash('sha256').update(token).digest('hex'):undefined;
  const grants=new Map(saved?.grants??[]);
  const prune=()=>{for(const [key,value] of grants)if(value.until<=now())grants.delete(key);};
  const credentials=createCredentials({now,lifetime,maxLifetime,capacity,saved:saved?.credentials,
    encode:({controller,...value})=>value,decode:value=>({...value,controller:new AbortController()}),
    onRevoke:session=>session.controller.abort()});
  const issueSession=grant=>credentials.issue({...grant,controller:new AbortController(),clientId:'mcp-'+randomBytes(16).toString('hex')},{until:grant.until});
  return {
    issue(principal,subject) {
      if(!/^did:key:[A-Za-z0-9]+$/.test(subject))throw Error('Invalid space');
      const until=Math.min(now()+maxLifetime,principal.connectionExpiresAt??Infinity);
      if(until<=now())throw Error('Connection expired');
      prune();if(grants.size>=capacity)throw Error('Connection capacity reached');
      const grant={principal,subject,until},resumeToken=randomBytes(32).toString('base64url');
      const session=issueSession(grant);grants.set(hash(resumeToken),grant);
      return {...session,resumeToken};
    },
    async fetch(request) {
      const cors={'access-control-allow-origin':'*','cache-control':'no-store','vary':'Origin'};
      if(request.method==='OPTIONS')return new Response(null,{status:204,headers:{...cors,'access-control-allow-methods':'GET, HEAD, POST, PUT, PATCH, DELETE, OPTIONS','access-control-allow-headers':'Authorization, Content-Type, Accept'}});
      const token=request.headers.get('authorization')?.match(/^Bearer ([A-Za-z0-9_-]{43})$/)?.[1];
      const url=new URL(request.url);
      if(url.pathname==='/space-api/session/resume'){
        if(request.method!=='POST')return new Response(null,{status:405,headers:cors});
        prune();const grant=grants.get(hash(token));
        if(!grant)return new Response(null,{status:401,headers:cors});
        return Response.json(issueSession(grant),{headers:cors});
      }
      if(url.pathname==='/space-api/session/renew'){
        if(request.method!=='POST')return new Response(null,{status:405,headers:cors});
        const renewed=credentials.renew(token);
        return Response.json(renewed??{error:'invalid_token'},{status:renewed?200:401,headers:cors});
      }
      const session=credentials.get(token);
      if(!session)return new Response(null,{status:401,headers:cors});
      // Decode components independently; encoded slashes cannot change scope.
      let parts;
      try{parts=url.pathname.split('/').map(decodeURIComponent);}catch{return new Response(null,{status:400,headers:cors});}
      if(parts.length<5||parts[1]!=='space-api'||parts[2]!=='api'||parts[3]!=='repository'||parts[4]!==session.subject||parts.slice(5).some(part=>!part||part==='.'||part==='..'||/[\\/\x00-\x1f]/.test(part))||(parts[5]==='branch'&&parts[6]!=='main')||!['GET','HEAD','POST','PUT','PATCH','DELETE'].includes(request.method))
        return new Response(null,{status:403,headers:cors});
      const worker=await resolveWorker(session.principal,session.subject);
      const path='/api/repository/'+encodeURIComponent(session.subject)+(parts.length>5?'/'+parts.slice(5).map(encodeURIComponent).join('/'):'')+url.search;
      const headers=new Headers({'x-tonk-client-id':session.clientId});
      for(const key of ['accept','content-type'])if(request.headers.has(key))headers.set(key,request.headers.get(key));
      const response=await worker.request(path,{method:request.method,headers,body:['GET','HEAD'].includes(request.method)?undefined:request.body,signal:AbortSignal.any([request.signal,session.controller.signal,AbortSignal.timeout(Math.max(1,session.until-now()))]),duplex:'half'});
      // Do not buffer: ordinary worker SSE subscriptions remain streams.
      const output=new Headers(cors);
      if(response.headers.has('content-type'))output.set('content-type',response.headers.get('content-type'));
      return new Response(response.body,{status:response.status,headers:output});
    },
    retainedTenants(){prune();return new Set([...grants.values()].map(grant=>grant.principal.tenantId).concat(credentials.snapshot().map(entry=>entry.value.principal.tenantId)));},
    snapshot(){prune();return {credentials:credentials.snapshot(),grants:[...grants]};},
    revokePrincipal(principal){
      const matches=session=>session.principal.tenantId===principal.tenantId&&session.principal.deviceDid===principal.deviceDid;
      credentials.revokeWhere(matches);for(const [key,grant] of grants)if(matches(grant))grants.delete(key);
    },
    close(){credentials.close();},
  };
}
