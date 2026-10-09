import {spawn} from 'node:child_process';
import {randomBytes} from 'node:crypto';
import {once} from 'node:events';

// One private full Tonk worker. The URL and token are host-only and never
// exposed in tool results. An outer space session selects permitted routes.
export async function startWorker({binary,dataDirectory}) {
  const token=randomBytes(32).toString('base64url');
  const child=spawn(binary,[dataDirectory],{stdio:['ignore','pipe','pipe'],env:{PATH:process.env.PATH,TONK_WORKER_HOST_TOKEN:token,DO_NOT_TRACK:'1'}});
  child.stderr.resume();
  let buffer='';
  const address=await new Promise((resolve,reject)=>{
    const timer=setTimeout(()=>{child.kill();reject(Error('Worker startup timed out'));},60000);
    child.once('error',()=>{clearTimeout(timer);reject(Error('Worker failed to start'));});
    child.once('exit',()=>{clearTimeout(timer);reject(Error('Worker stopped'));});
    child.stdout.setEncoding('utf8');
    child.stdout.on('data',chunk=>{
      buffer+=chunk;
      if(buffer.length>262144){child.kill();clearTimeout(timer);reject(Error('Invalid worker startup'));return;}
      for(let end;(end=buffer.indexOf('\n'))>=0;){
        const line=buffer.slice(0,end);buffer=buffer.slice(end+1);
        try{const value=JSON.parse(line);if(/^127\.0\.0\.1:\d+$/.test(value.address)){clearTimeout(timer);resolve(value.address);}}catch{}
      }
    });
  });
  let closed=false;
  const alive=()=>!closed&&child.exitCode===null&&child.signalCode===null&&!child.killed;
  child.on('exit',(code,signal)=>{if(!closed)console.error(JSON.stringify({event:'tonk-worker-exit',code,signal}));});
  return {
    isRunning:alive,
    async request(path,options={}) {
      if(!alive()||!path.startsWith('/')||path.startsWith('//'))throw Error('Worker unavailable');
      const url=new URL(path,'http://'+address);
      if(url.origin!=='http://'+address)throw Error('Invalid worker path');
      const headers=new Headers(options.headers);headers.set('authorization','Bearer '+token);
      return fetch(url,{...options,headers,redirect:'error'});
    },
    async close(){if(closed)return;closed=true;const exited=child.exitCode!==null||child.signalCode!==null?Promise.resolve():once(child,'exit');child.kill();await exited;},
  };
}
