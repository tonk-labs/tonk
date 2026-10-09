// Separate from native seed checkpoints: small, conditional, write-through state.
// Fail closed after an ambiguous write; never serve uncommitted credentials.
export async function createDurableState({fetch:request=globalThis.fetch,endpoint='http://checkpoint.internal/credentials',limit=2*1024*1024}={}){
 let etag,previous,failed=false,queue=Promise.resolve();
 const response=await request(endpoint,{redirect:'error',signal:AbortSignal.timeout(30000)});
 let state;
 if(response.status!==404){
  if(!response.ok||!response.headers.get('etag'))throw Error('Credential restore failed');
  const reader=response.body.getReader();let size=0;const chunks=[];
  for(;;){const {done,value}=await reader.read();if(done)break;size+=value.length;if(size>limit){await reader.cancel();throw Error('Credential state exceeds limit');}chunks.push(value);}
  previous=Buffer.concat(chunks).toString();state=JSON.parse(previous);etag=response.headers.get('etag');
 }
 return {state,run(operation,snapshot){
  const next=queue.then(async()=>{
   if(failed)throw Error('Credential persistence unavailable');
   const result=await operation();
   const body=JSON.stringify(snapshot());
   if(body!==previous){
    try{
     if(Buffer.byteLength(body)>limit)throw Error('Credential state exceeds limit');
     const saved=await request(endpoint,{method:'PUT',body,redirect:'error',signal:AbortSignal.timeout(30000),headers:{'content-type':'application/json',...(etag?{'if-match':etag}:{'if-none-match':'*'})}});
     if(!saved.ok||!saved.headers.get('etag'))throw Error('Credential save failed');
     etag=saved.headers.get('etag');previous=body;
    }catch(error){failed=true;throw error;}
   }
   return result;
  });queue=next.catch(()=>{});return next;
 }};
}
