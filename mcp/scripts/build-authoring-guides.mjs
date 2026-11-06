import { readFile, writeFile } from 'node:fs/promises';
const root = new URL('../../', import.meta.url);
const check = process.argv.includes('--check');
const rust = await readFile(new URL('rust/tonk-cli/src/guide.rs', root), 'utf8');
const sources = new Map(
  [...rust.matchAll(/pub const (\w+): &str = include_str!\("([^"]+)"\);/g)].map((m) => [
    m[1],
    m[2],
  ]),
);
const guides = {};
for (const match of rust.matchAll(/"([a-z-]+)" => Some\(([A-Z_]+)\)/g)) {
  const path = sources.get(match[2]);
  if (!path) throw Error('Missing guide source');
  guides[match[1]] = await readFile(new URL(path, new URL('rust/tonk-cli/src/', root)), 'utf8');
}
const instructions = guides.agent;
if (!instructions) throw Error('Missing canonical agent guide');
const output =
  '// Generated from canonical CLI manuals. Do not edit.\n' +
  'export const guides = ' +
  JSON.stringify(guides) +
  ';\n' +
  'export const instructions = ' +
  JSON.stringify(instructions) +
  ';\n';
const target = new URL('mcp/authoring-content.mjs', root);
if (check) {
  if ((await readFile(target, 'utf8')) !== output)
    throw Error('Authoring bundle is stale; run scripts/build-authoring-guides.mjs.');
} else await writeFile(target, output);
