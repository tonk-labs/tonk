import {mkdtemp,realpath,lstat,cp} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {basename,dirname,join} from 'node:path';
import {TonkToolError} from './core.mjs';
import {accountSpaces} from './account-spaces.mjs';
import {startWorker} from './worker-process.mjs';

async function json(worker,path,body) {
  const response=await worker.request(path,body===undefined?{}:{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(body)});
  if(!response.ok)throw new TonkToolError('Tonk '+(path.startsWith('/api/repository/')?'space loading':'account setup')+' failed (HTTP '+response.status+'). This does not by itself mean your sign-in expired.');
  return response.json();
}
export function workerAccountProvisioner({binary,dataRoot,start=startWorker}) {
  return async()=>{
    const directory=await mkdtemp(join(dataRoot,'tenant-'));
    const worker=await start({binary,dataDirectory:directory});
    let deviceDid;
    try{deviceDid=(await json(worker,'/api/identify')).did;}finally{await worker.close();}
    return {tenantId:basename(directory),deviceDid,async authorize(authorization){
      // This development host only attaches to its configured Tonk provider.
      let stage='provider';
      let worker;
      try {
      const remote=new URL(authorization.remote);
      if(remote.origin!=='https://tonk.foundation'||remote.username||remote.password)throw Error('Unexpected account provider');
      stage='worker-start';
      worker=await start({binary,dataDirectory:directory});
      stage='device-check';
        if((await json(worker,'/api/identify')).did!==deviceDid)throw Error('Device changed');
        stage='root-import';
        const root=await json(worker,'/api/identity/root',{credentialId:authorization.credentialId||'',delegationHex:authorization.delegationHex});
        if(root.status!=='ready'||root.deviceDid!==deviceDid)throw Error('Invalid account root');
        stage='account-attach';
        const attached=await json(worker,'/api/account/attach',{provider:remote.origin,rootDid:root.rootDid,credentialId:authorization.credentialId||'',delegationHex:authorization.delegationHex,remote:remote.href});
        stage='account-hydration';
        if(attached.status!=='registered'||attached.rootDid!==root.rootDid||attached.deviceDid!==deviceDid||attached.accountState!=='ready')throw Error('Account hydration did not complete');
        return {rootDid:root.rootDid,deviceDid};
      }catch{
        // Never attach the upstream error, response body or supplied grant.
        throw Object.assign(Error('Account connection failed'),{authorizationStage:stage});
      }finally{await worker?.close();}
    }};
  };
}
export function createWorkerTenants({binary,dataRoot,capacity=16,start=startWorker}) {
  const entries=new Map();
  return {
    async account(principal) {
      if(!/^tenant-[A-Za-z0-9]{6}$/.test(principal.tenantId))throw Error('Invalid tenant selection');
      let entry=entries.get(principal.tenantId);
      if(entry){if(entry.rootDid!==principal.rootDid||entry.deviceDid!==principal.deviceDid)throw Error('Tenant binding changed');return entry.worker;}
      if(entries.size>=capacity)throw new TonkToolError('The hosted runtime has reached its account-worker limit. Reconnecting will not resolve this; the service needs capacity recovery.');
      const root=await realpath(dataRoot),directory=join(root,principal.tenantId);
      if((await lstat(directory)).isSymbolicLink()||dirname(await realpath(directory))!==root)throw Error('Invalid tenant directory');
      entry={rootDid:principal.rootDid,deviceDid:principal.deviceDid};entries.set(principal.tenantId,entry);
      entry.worker=(async()=>{
        // Runtime caches are separate from the immutable authorization seed.
        // Checkpointing a login never copies a live worker's open storage.
        const live=await mkdtemp(join(tmpdir(),'tonk-worker-live-'));
        await cp(directory,live,{recursive:true});
        let worker=await start({binary,dataDirectory:live}),restarting,closed=false;
        async function ensureWorker(){
          if(closed)throw new TonkToolError('Tonk worker is shutting down. Retry a read after it is ready.');
          if(restarting)return restarting;
          if(worker.isRunning?.()!==false)return worker;
          restarting=(async()=>{
            const next=await start({binary,dataDirectory:live});
            try{const identity=await json(next,'/api/identity/root');if(identity.rootDid!==principal.rootDid||identity.deviceDid!==principal.deviceDid)throw Error('Account binding mismatch');}
            catch(error){await next.close();throw error;}
            worker=next;return worker;
          })().finally(()=>{restarting=undefined;});
          return restarting;
        }
        try{
          const identity=await json(worker,'/api/identity/root');
          if(identity.rootDid!==principal.rootDid||identity.deviceDid!==principal.deviceDid)throw Error('Account binding mismatch');

          return {
            close:async()=>{closed=true;await restarting?.catch(()=>{});await worker.close();},
            async request(path,options={}) {
              const current=await ensureWorker();
              let response;
              try{response=await current.request(path,options);}
              catch{
                // Never replay a request: a write may have committed before exit.
                throw new TonkToolError('The Tonk worker request was interrupted. Your sign-in may still be valid. Read the space on the next call; do not repeat an uncertain write.');
              }
              const url=new URL(path,'http://worker');
              const match=url.pathname.match(/^\/api\/repository\/([^/]+)\/branch\/main\/(transact|evaluate)$/);
              const prefix=match?'/api/repository/'+match[1]+'/branch/main':undefined;
              const writes=!!match&&options.method==='POST'&&(url.pathname===prefix+'/transact'||(url.pathname===prefix+'/evaluate'&&url.searchParams.get('transact')!=='false'));
              if(writes&&response.ok){
                // Await normal worker command dispatch, then publish once. A
                // failed push is uncertain: never replay the user's command.
                const pushed=await current.request(prefix+'/sync/push',{method:'POST'});
                if(!pushed.ok)return Response.json({error:'Applied locally; remote push could not be confirmed. Read before retrying.'},{status:503});
              }
              return response;
            },
          };
        }catch(error){await worker.close();throw error;}
      })().catch(error=>{if(entries.get(principal.tenantId)===entry)entries.delete(principal.tenantId);throw error;});
      return entry.worker;
    },
    async list(principal){return accountSpaces(await this.account(principal));},
    async resolve(principal,subject){
      if(!/^did:key:[A-Za-z0-9]+$/.test(subject))throw Error('Invalid selected space');
      const worker=await this.account(principal);
      if(!(await accountSpaces(worker)).some(space=>space.subject===subject))throw new TonkToolError('That space is not in the current account catalog. Call tonk_list_spaces and use an exact returned subject.');
      await json(worker,'/api/repository/'+encodeURIComponent(subject));
      return worker;
    },
    async close(){const values=[...entries.values()];entries.clear();await Promise.all(values.map(async entry=>(await entry.worker).close()));},
  };
}
