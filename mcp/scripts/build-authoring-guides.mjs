import {readFile,writeFile} from 'node:fs/promises';
const root=new URL('../../',import.meta.url);
const rust=await readFile(new URL('rust/tonk-cli/src/guide.rs',root),'utf8');
const sources=new Map([...rust.matchAll(/pub const (\w+): &str = include_str!\("([^"]+)"\);/g)].map(m=>[m[1],m[2]]));
const topics={};
for(const match of rust.matchAll(/"([a-z-]+)" => Some\(([A-Z_]+)\)/g)){
 const path=sources.get(match[2]);if(!path)throw Error('Missing guide source');
 topics[match[1]]=await readFile(new URL(path,new URL('rust/tonk-cli/src/',root)),'utf8');
}
await writeFile(new URL('mcp/authoring-content.mjs',root),'// Generated from canonical CLI manuals by scripts/build-authoring-guides.mjs.\nexport const guides='+JSON.stringify(topics)+';\n');
