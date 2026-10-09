// Pin the public Tonk guest runtime; private space data never enters this build.
import { mkdir, writeFile, readFile } from 'node:fs/promises';
const origin = 'https://tonk.foundation';
const manifest = {js:'guest-09762df49c1a18fa.js',wasm:'guest_bg-27f62bbc57e37723.wasm',waJs:'wa-eb4fe8aa4c918245.js',waCss:'wa-ec3476116cae89ee.css'};
async function get(path, binary=false) {
  const response = await fetch(origin+path, {redirect:'error'});
  if (!response.ok) throw Error(`Asset fetch failed: ${path}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  if (bytes.length > 20*1024*1024) throw Error('Asset too large');
  return binary ? bytes.toString('base64') : bytes.toString();
}
const cached=process.env.TONK_GUEST_PACK?JSON.parse(await readFile(process.env.TONK_GUEST_PACK,'utf8')):null;
if(cached&&JSON.stringify(cached.manifest)!==JSON.stringify(manifest))throw Error('Runtime manifest mismatch');
const glue=cached?cached.glue:await get('/guest/'+manifest.js), snippets=[];
if(glue.trimStart().startsWith('<'))throw Error('Guest glue URL returned HTML');
for (const match of glue.matchAll(/import\s+[^;]+?from\s*['"]([^'"]+snippets\/[^'"]+)['"];?/g)) {
  const path=match[1].replace(/^\.\//,'');
  if(path.includes('..'))throw Error('Unexpected snippet path');
  snippets.push({stmt:match[0],src:cached?cached.snippets.find(s=>s.stmt===match[0])?.src:await get('/guest/'+path)});
}
const local = path => readFile(new URL('../../'+path,import.meta.url),'utf8');
const graph = async (directory, entry, seen=new Map()) => {
  if(seen.has(entry))return [];
  seen.set(entry,true); const src=await local(directory+'/'+entry), result=[];
  for(const match of src.matchAll(/['"]\.\/([^'"$]+\.js)['"]/g)) result.push(...await graph(directory,match[1],seen));
  result.push({name:entry,src});return result;
};
// Match tonk-portal's build_inject_payload: base CSS, application CSS, then
// inline local fonts so opaque guests need no external font requests.
const waCss=cached?(cached.waCss??cached.css):await get('/guest/'+manifest.waCss);
let css=waCss+'\n'+await local('rust/tonk-ui/styles.css');
for(const match of [...css.matchAll(/url\(["']?(\/fonts\/([^"')]+))["']?\)/g)]) {
  const name=match[2];
  if(!/^[A-Za-z0-9_.-]+\.(woff2?|otf|ttf)$/.test(name))throw Error('Invalid font asset');
  const mime=name.endsWith('.woff2')?'font/woff2':name.endsWith('.woff')?'font/woff':name.endsWith('.otf')?'font/otf':'font/ttf';
  const font=await readFile(new URL('../../rust/tonk-ui/assets/fonts/'+name,import.meta.url));
  css=css.replaceAll(match[0],`url("data:${mime};base64,${font.toString('base64')}")`);
}
const theme=await local('rust/tonk-ui/assets/space-theme.js');
const pack={manifest,glue,snippets,wasm:cached?cached.wasm:await get('/guest/'+manifest.wasm,true),wa:cached?cached.wa:await get('/guest/'+manifest.waJs,true),waCss,css,theme,
  bootstrap:await local('rust/tonk-portal/src/bootstrap.js'),runtimeBootstrap:await local('rust/tonk-portal/src/runtime_bootstrap.js'),
  prose:await graph('rust/tonk-prose/assets','tonk-prose.js'),proseCore:await graph('rust/tonk-prose/assets','tonk-prose-editor.js'),
  table:[{name:'tonk-table.js',src:await local('rust/tonk-table/assets/tonk-table.js')}],
  tableGrid:await Promise.all(['tonk-table-engine.js','tonk-table-grid.js'].map(async name=>({name,src:await local('rust/tonk-table/assets/'+name)}))),
  code:await graph('rust/tonk-code/assets','tonk-code.js')};
if(!Buffer.from(pack.wasm,'base64').subarray(0,4).equals(Buffer.from([0,97,115,109])))throw Error('Guest Wasm URL did not return a Wasm binary');
if(snippets.some(s=>typeof s.src!=='string'||s.src.trimStart().startsWith('<')))throw Error('Invalid guest snippet');
await mkdir(new URL('../public/',import.meta.url),{recursive:true});
await writeFile(new URL('../public/space-runtime.json',import.meta.url),JSON.stringify(pack));
console.log(`Built real guest runtime: ${snippets.length} snippets, ${pack.code.length} code modules`);

const guest = `<!doctype html><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval' blob:; style-src 'unsafe-inline' blob:; img-src data: blob:; font-src data: blob:; connect-src blob:; frame-src blob: about:; form-action 'none'"><style>html,body{margin:0;min-height:100%;font-family:system-ui}</style>`;
const script = source => '<script>'+source.replace(/<\/script/gi,'<\\/script')+'</script>';
const mount = `addEventListener('message',e=>{if(e.source!==parent||e.data?.type!=='tonk-mount')return;const display=document.createElement('tonk-display');display.setAttribute('entity',e.data.site);display.setAttribute('model',e.data.model);display.setAttribute('with','main@'+e.data.subject);document.body.replaceChildren(display);});
let lastTitle,editable=false;addEventListener('message',e=>{if(e.source===parent&&e.data?.type==='tonk-edit'){editable=e.data.enabled===true;lock();}});const lock=()=>{document.querySelectorAll('tonk-prose').forEach(el=>el.toggleAttribute('readonly',!editable));document.querySelectorAll('tonk-code').forEach(el=>el.setAttribute('readonly',''));const title=document.querySelector('tonk-notebook[title]')?.getAttribute('title');if(title&&title!==lastTitle){lastTitle=title;window.tonk?.setTitle?.(title);}};new MutationObserver(lock).observe(document,{childList:true,subtree:true,attributes:true,attributeFilter:['title']});document.addEventListener('beforeinput',e=>{if(!editable)e.preventDefault();},true);`;
await writeFile(new URL('../public/space-guest.html',import.meta.url),guest+script(mount)+script(pack.bootstrap)+script(pack.runtimeBootstrap)+'<body></body>');

// Full worker host: no component-specific edit locks or command adapters.
const workerMount = await local('rust/tonk-portal/src/embed_bootstrap.js');
await writeFile(new URL('../public/worker-guest.html',import.meta.url),guest+script(theme)+script(workerMount)+script(pack.bootstrap)+script(pack.runtimeBootstrap)+'<body></body>');
