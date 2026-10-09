import {guides} from './authoring-content.mjs';
import {TonkToolError} from './core.mjs';
export function authoringGuide({topic}={}){
 if(!topic)return {topics:Object.keys(guides),usage:'Read notation before defining schema, views for rendering, and events for editable controls. These are the canonical CLI manuals. In MCP send inline notation to tonk_query (read-only) or tonk_evaluate (commit); CLI commands shown in manuals are not tool inputs. Use the explicit space subject on those calls. Open /<concept> for its directory; preserve existing home/content unless asked to change it.'};
 if(!Object.hasOwn(guides,topic))throw new TonkToolError('Unknown guide; omit topic to list available manuals.');
 return {topic,source:'Canonical Tonk CLI manual bundled with this connector',text:guides[topic]};
}
