// Local visual harness: synthetic data only, no credentials or remote calls.
import { createServer } from 'node:http';
import { notebookHTML } from '../ui.mjs';
const html = `<!doctype html><meta charset="utf-8"><title>Tonk notebook preview test</title>
<style>body{font:15px system-ui;background:#ecece8;margin:30px}iframe{width:min(760px,100%);height:660px;border:1px solid #ccc;border-radius:14px;background:#fafaf7}#checks{white-space:pre-wrap}</style>
<h1>Notebook widget test</h1><p>Synthetic data. No connection to a real space.</p><p id="checks">Waiting for the renderer…</p><iframe title="Tonk notebook widget" sandbox="allow-scripts allow-same-origin"></iframe>
<script>
const frame=document.querySelector('iframe'); let reads=0;
const notebook={entity:'urn:notebook:preview',title:'A notebook from ChatGPT',blocks:[{position:'N1',entity:'urn:block:first',source:'# A working notebook'},{position:'N9',entity:'urn:block:last',source:'Ordered text with **bold**, a list, and a query cell.'}],unplacedBlocks:0,url:'https://tonk.foundation/space/did%3Akey%3Az6Test/notebook/urn%3Anotebook%3Apreview',markdown:'# A working notebook\\n\\nOrdered text with **bold** and a list.\\n\\n- First block\\n- Second block\\n\\n\`\`\`dialog-yaml\\nnotebook/named:\\n\`\`\`',readOnly:true};
const send=(message)=>frame.contentWindow.postMessage({jsonrpc:'2.0',...message},'*');
addEventListener('message',event=>{if(event.source!==frame.contentWindow)return;const m=event.data;if(m.method==='ui/initialize')send({id:m.id,result:{protocolVersion:'2026-01-26',hostInfo:{name:'visual-test',version:'1'},hostCapabilities:{},hostContext:{}}});else if(m.method==='ui/notifications/initialized')send({method:'ui/notifications/tool-result',params:{structuredContent:notebook}});else if(m.method==='tools/call'){if(m.params.name!=='tonk_show_notebook'||m.params.arguments.entity!==notebook.entity)throw Error('Unexpected tool call');reads++;send({id:m.id,result:{structuredContent:{...notebook,markdown:notebook.markdown+'\\n\\nRefreshed from Tonk.'}}});}});
frame.srcdoc=${JSON.stringify(notebookHTML).replace(/</g,'\\u003c')};
setInterval(()=>{const doc=frame.contentDocument,editor=doc?.querySelector('.ProseMirror');if(!editor)return;const strong=editor.querySelector('strong');const link=doc.querySelector('#open');document.querySelector('#checks').textContent='Renderer: '+(strong?.textContent==='bold'?'rich text passed':'FAILED')+'\\nRead-only: '+(editor.getAttribute('contenteditable')==='false'?'passed':'FAILED')+'\\nOpen in Tonk: '+(!link.hidden&&link.href===notebook.url?'passed':'FAILED')+'\\nRefresh reads: '+reads+'\\nRefresh content: '+(editor.textContent.includes('Refreshed from Tonk.')?'passed':'click Refresh');},200);
</script>`;
const server=createServer((request,response)=>{if(request.url!=='/'){response.writeHead(404).end();return;}response.writeHead(200,{'content-type':'text/html; charset=utf-8','cache-control':'no-store'}).end(html);});
server.listen(8794,'127.0.0.1',()=>console.log('Notebook visual harness: http://127.0.0.1:8794'));
