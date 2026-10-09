import {TonkToolError} from './core.mjs';
// Same profile catalog query used by the desktop runtime.
export async function accountSpaces(worker){
 const probe=await worker.request('/api/profile/repository');
 if(!probe.ok&&![404,405].includes(probe.status))throw new TonkToolError('Account profile unavailable (HTTP '+probe.status+'). Retry this read; reconnecting is not yet indicated.');
 const prefix=probe.ok?'/api/profile/branch/':'/api/repository/profile:tonk/branch/';
 const field=(the,as)=>({the,as,cardinality:'one'}),variable=name=>({'?':{name}});
 const query=async(branch,body)=>{
  const response=await worker.request(prefix+encodeURIComponent(branch)+'/query',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(body)});
  if(!response.ok)throw new TonkToolError('Account space catalog unavailable (HTTP '+response.status+'). Retry this read; reconnecting is not yet indicated.');
  const rows=await response.json();if(!Array.isArray(rows))throw Error('Invalid account catalog');return rows;
 };
 const active=await query('meta',{predicate:{with:{branch:field('tonk.dialog.replica/active-branch','Entity')}},terms:{this:variable('this'),branch:variable('branch')}});
 let branch='main';
 if(active[0]?.fields?.branch){
  const names=await query('meta',{predicate:{with:{name:field('xyz.tonk.branch/name','Text')}},terms:{this:active[0].fields.branch,name:variable('name')}});
  branch=names[0]?.fields?.name;if(typeof branch!=='string'||!branch)throw Error('Unknown account branch');
 }
 const rows=await query(branch,{predicate:{with:{subject:field('xyz.tonk.space/subject','Entity'),name:{...field('xyz.tonk.space/name','Text'),optional:true}}},terms:{this:variable('this'),subject:variable('subject'),name:variable('name')}});
 const spaces=new Map();for(const row of rows){const {subject,name}=row.fields??{};if(!/^did:key:[A-Za-z0-9]+$/.test(subject))throw Error('Invalid catalog subject');spaces.set(subject,{subject,name:typeof name==='string'?name:null});}
 return [...spaces.values()];
}
