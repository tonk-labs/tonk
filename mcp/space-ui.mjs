import { readFileSync } from 'node:fs';
import {readNotebook} from './notebook.mjs';
import {editNotebook} from './notebook-edit.mjs';
import * as z from 'zod/v4';
export const spaceURI = 'ui://tonk/space-v2.html';
export const spaceHTML = readFileSync(new URL('./ui/space.html', import.meta.url), 'utf8');
export function registerSpaceUI(server, backend) {
  if (!backend.uiRead) return;
  server.registerResource('tonk-space', spaceURI, {mimeType:'text/html;profile=mcp-app'}, async () => ({contents:[{
    uri:spaceURI,mimeType:'text/html;profile=mcp-app',text:spaceHTML,
    _meta:{ui:{prefersBorder:true,csp:{connectDomains:['https://tonk-mcp-test.tonk.workers.dev'],resourceDomains:[],frameDomains:['https://tonk-mcp-test.tonk.workers.dev']}},
      'openai/widgetCSP':{connect_domains:['https://tonk-mcp-test.tonk.workers.dev'],resource_domains:[],frame_domains:['https://tonk-mcp-test.tonk.workers.dev'],redirect_domains:['https://tonk.foundation']}}
  }]}));
  if(backend.capabilities.includes('tonk_apply')) {
    server.registerTool('tonk_ui_begin_edit', {
      description:'Read the current notebook revision before the user edits existing text in the embedded frontend.',
      inputSchema:z.object({entity:z.string().max(2048)}).strict(),
      annotations:{readOnlyHint:true,openWorldHint:false},_meta:{ui:{visibility:['app']}},
    }, async ({entity},context)=>{
      try{return {content:[],_meta:{notebook:await readNotebook(backend,entity,context.signal)}};}
      catch{return {isError:true,content:[{type:'text',text:'Could not begin editing this notebook.'}]};}
    });
    server.registerTool('tonk_ui_edit_notebook', {
      description:'Save an explicit user edit to existing notebook text or title. Conditional revision check and one remote push; no automatic retries.',
      inputSchema:z.object({entity:z.string().max(2048),request:z.record(z.string(),z.unknown()),expectedRevision:z.unknown()}).strict(),
      annotations:{readOnlyHint:false,destructiveHint:false,idempotentHint:false,openWorldHint:false},_meta:{ui:{visibility:['app']}},
    }, async (args,context)=>{
      try{return {content:[],_meta:{saved:await editNotebook(backend,args,context.signal)}};}
      catch{return {isError:true,content:[{type:'text',text:'The edit could not be confirmed. Copy your text before refreshing. Do not retry automatically; read the notebook to check whether it saved.'}]};}
    });
  }
  const path=z.string().min(1).max(4096);
  server.registerTool('tonk_open_space', {
    description:'Open the actual Tonk space frontend using its installed views and routes. Opens read-only; users can choose Edit text to update existing notebook text. Path is within the connected space, for example / or /notebook/<entity>. No separate sign-in or desktop app. Block insertion, deletion and account-level navigation are not enabled.',
    inputSchema:z.object({path:path.default('/')}).strict(),annotations:{readOnlyHint:true,openWorldHint:false},_meta:{ui:{resourceUri:spaceURI}},
  }, async ({path},context)=>{
    try { const route=await backend.uiRead({path},context.signal); return {content:[{type:'text',text:'Opened the Tonk space frontend in read-only mode.'}],structuredContent:route}; }
    catch {return {isError:true,content:[{type:'text',text:'Could not open this space route. Check the path and reconnect if necessary.'}]};}
  });
  server.registerTool('tonk_ui_read', {
    description:'Read-only transport for the embedded Tonk renderer. Queries are restricted to the connected space and main branch.',
    inputSchema:z.object({path,queries:z.array(z.record(z.string(),z.unknown())).min(1).max(32)}).strict(),
    annotations:{readOnlyHint:true,openWorldHint:false},_meta:{ui:{visibility:['app']}},
  },async ({path,queries},context)=>{
    try {
      const rows=[];
      let size = 0;
      for (const query of queries) {
        const result = (await backend.uiRead({path,query},context.signal)).rows;
        size += Buffer.byteLength(JSON.stringify(result));
        if (size > 1_000_000) throw Error('Render batch too large');
        rows.push(result);
      }
      return {content:[],_meta:{rows}};
    } catch {return {isError:true,content:[{type:'text',text:'The space renderer could not read this query. No changes were made.'}]};}
  });
}
