import {isDeepStrictEqual} from 'node:util';
import {readNotebook, isEntity} from './notebook.mjs';

// Adapt only the existing notebook's text/rename commands to canonical durable
// writes. No arbitrary transient command, block insertion, or target override.
export async function editNotebook(backend,{entity,request,expectedRevision},signal) {
  const snapshot=await readNotebook(backend,entity,signal);
  if(!isDeepStrictEqual(snapshot.revision,expectedRevision)) throw Error('The space changed. Refresh before editing again.');
  if(!request||Object.keys(request).some(k=>k!=='claims')||!Array.isArray(request.claims)||request.claims.length<1||request.claims.length>16) throw Error('Unsupported notebook edit.');
  const documents=[];
  for(const claim of request.claims){
    if(claim.op!=='assert')throw Error('Only existing text and title edits are supported.');
    const app=claim.application, fields=app?.predicate?.concept?.with, args=app?.parameters;
    if(app?.predicate?.kind!=='transient'||!fields||!args)throw Error('Unsupported notebook command.');
    const values={};
    for(const [key,descriptor] of Object.entries(fields)){
      if(!descriptor?.the||Object.hasOwn(values,descriptor.the))throw Error('Unsupported command field.');
      values[descriptor.the]=args[key];
    }
    const keys=Object.keys(values).sort();
    if(isDeepStrictEqual(keys,['xyz.tonk.block.edit/notebook','xyz.tonk.block.edit/source','xyz.tonk.block.edit/subject'])){
      const block=values['xyz.tonk.block.edit/subject'], source=values['xyz.tonk.block.edit/source'];
      if(values['xyz.tonk.block.edit/notebook']!==entity||!isEntity(block)||!snapshot.blocks.some(b=>b.entity===block)||typeof source!=='string'||source.length>16000)throw Error('Edit must name an existing block in this notebook.');
      documents.push(`notebook/block!:\n  this: ${block}\n  notebook: ${entity}\n  source: ${JSON.stringify(source).replaceAll('!', '\\u0021')}\n`);
    }else if(isDeepStrictEqual(keys,['xyz.tonk.notebook.retitle/subject','xyz.tonk.notebook.retitle/title'])){
      const title=values['xyz.tonk.notebook.retitle/title'];
      if(values['xyz.tonk.notebook.retitle/subject']!==entity||typeof title!=='string'||title.length>500)throw Error('Invalid notebook title.');
      documents.push(`notebook/named!:\n  this: ${entity}\n  title: ${JSON.stringify(title).replaceAll('!', '\\u0021')}\n`);
    }else throw Error('Only existing text and title edits are supported.');
  }
  const document=documents.join('\n');
  const preview=await backend.call('tonk_preview',{document},signal);
  if(!isDeepStrictEqual(preview.revision,expectedRevision))throw Error('The space changed. Refresh before editing again.');
  // One conditional apply only. In particular, never retry an uncertain push.
  return backend.call('tonk_apply',{document,expectedRevision},signal);
}
