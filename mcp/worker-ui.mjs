import {readFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
import * as z from 'zod/v4';
const html=readFileSync(new URL('./ui/worker-space.html',import.meta.url),'utf8');
// Hosts may cache resource HTML by URI across tool calls.
const uri=`ui://tonk/worker-space-${createHash('sha256').update(html).digest('hex').slice(0,16)}.html`;
export function registerWorkerUI(server,backend) {
  if(!backend.openSpace)return;
  server.registerResource('tonk-space',uri,{mimeType:'text/html;profile=mcp-app'},async()=>({contents:[{
    uri,mimeType:'text/html;profile=mcp-app',text:html,
    _meta:{ui:{csp:{connectDomains:['https://tonk-mcp-test.tonk.workers.dev'],frameDomains:['https://tonk-mcp-test.tonk.workers.dev']}},
      'openai/widgetCSP':{connect_domains:['https://tonk-mcp-test.tonk.workers.dev'],frame_domains:['https://tonk-mcp-test.tonk.workers.dev'],resource_domains:[]}},
  }]}));
  server.registerTool('tonk_open_space',{
    description:'Open the connected Tonk space in its normal frontend. Use a path within this space, such as / or /notebook/<entity>. Entity route parameters must be actual entity URIs returned by tonk_query, never titles or names. For a notebook requested by title, query notebook/named to obtain its this entity, then open /notebook/<that entity>. Opening the frontend does not verify that a route rendered successfully. The user and agent operate on the same worker.',
    inputSchema:z.object({...(backend.spaceSelection?{space:z.string().regex(/^did:key:[A-Za-z0-9]+$/).describe('Exact subject from tonk_list_spaces.')} : {}),path:z.string().min(1).max(4096).default('/')}).strict(),
    annotations:{readOnlyHint:true,openWorldHint:false},_meta:{ui:{resourceUri:uri}},
  },async({path,space})=>{
    if(!path.startsWith('/')||path.startsWith('//')||/[\\\x00-\x1f]/.test(path))return {isError:true,content:[{type:'text',text:'Provide a path within the connected space.'}]};
    const session=await backend.openSpace(space);
    return {content:[{type:'text',text:'Requested the Tonk frontend at the supplied path. Route rendering is not verified by this tool response.'}],structuredContent:{subject:space??backend.subject,path},_meta:{session}};
  });
}
