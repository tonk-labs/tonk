import {TonkToolError} from './core.mjs';

// Both MCP and the embedded frontend are clients of the ordinary worker.
// Authority is supplied by the host, never by tool arguments or document data.
export function workerBackend({subject, request}) {
  if (!/^did:key:[A-Za-z0-9]+$/.test(subject)) throw Error('Invalid selected space');
  const base='/api/repository/'+encodeURIComponent(subject)+'/branch/main';
  return {
    subject,
    capabilities:['tonk_query','tonk_evaluate','tonk_space_info'],
    async call(name,args,signal) {
      if(name==='tonk_space_info') {
        if(Object.keys(args).length) throw new TonkToolError('No target arguments are accepted.');
        return {subject,branches:['main']};
      }
      if(!['tonk_query','tonk_evaluate'].includes(name)) throw new TonkToolError('Tool unavailable.');
      if(!args||Object.keys(args).length!==1||typeof args.document!=='string'||!args.document.trim()||Buffer.byteLength(args.document)>32000)
        throw new TonkToolError('Provide one inline notation document, up to 32 KB.');
      // Request performs exactly one call. A network failure after evaluation
      // is an uncertain write, never permission to replay the document.
      const response=await request(base+'/evaluate?transact='+(name==='tonk_evaluate'),{
        method:'POST',headers:{'content-type':'text/plain'},body:args.document,signal,
      });
      if(!response.ok && response.status===400){
        const error=(await response.json().catch(()=>null))?.error;
        if(error?.kind==='analyze'&&typeof error.message==='string')throw new TonkToolError('Notation rejected: '+error.message.slice(0,2000));
      }
      if(!response.ok) throw new TonkToolError(name==='tonk_query'
        ? 'The worker rejected this query.'
        : 'The worker could not confirm evaluation. Read the space before retrying; commands may already have run.');
      return response.json();
    },
  };
}
