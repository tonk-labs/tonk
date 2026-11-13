// Pin the public Tonk guest runtime; private space data never enters this build.
import { mkdir, writeFile, readFile } from 'node:fs/promises';
import { join } from 'node:path';
// CI consumes the same guest build as its matching Tonk deployment.
const distribution = process.env.TONK_UI_DIST;
const cached =
  !distribution && process.env.TONK_GUEST_PACK
    ? JSON.parse(await readFile(process.env.TONK_GUEST_PACK, 'utf8'))
    : null;
if (!distribution && !cached) throw Error('Set TONK_UI_DIST or TONK_GUEST_PACK.');
const manifest = distribution
  ? JSON.parse(await readFile(join(distribution, 'guest/manifest.json'), 'utf8'))
  : cached.manifest;
for (const key of ['js', 'wasm', 'waJs', 'waCss']) {
  if (typeof manifest?.[key] !== 'string' || !/^[A-Za-z0-9_.-]+$/.test(manifest[key]))
    throw Error('Invalid guest manifest');
}
async function get(path, binary = false) {
  const relative = path.replace(/^\/guest\//, '');
  if (!distribution || relative.includes('..') || relative.startsWith('/'))
    throw Error('Invalid local guest asset path');
  const bytes = await readFile(join(distribution, 'guest', relative));
  if (bytes.length > 20 * 1024 * 1024) throw Error('Asset too large');
  return binary ? bytes.toString('base64') : bytes.toString();
}
const glue = cached ? cached.glue : await get('/guest/' + manifest.js),
  snippets = [];
if (glue.trimStart().startsWith('<')) throw Error('Guest glue URL returned HTML');
for (const match of glue.matchAll(/import\s+[^;]+?from\s*['"]([^'"]+snippets\/[^'"]+)['"];?/g)) {
  const path = match[1].replace(/^\.\//, '');
  if (path.includes('..')) throw Error('Unexpected snippet path');
  snippets.push({
    stmt: match[0],
    src: cached
      ? cached.snippets.find((s) => s.stmt === match[0])?.src
      : await get('/guest/' + path),
  });
}
const local = (path) => readFile(new URL('../../' + path, import.meta.url), 'utf8');
// Match tonk-site's build_inject_payload: base CSS, application CSS, then
// inline local fonts so opaque guests need no external font requests.
const waCss = cached ? (cached.waCss ?? cached.css) : await get('/guest/' + manifest.waCss);
let css = waCss + '\n' + (await local('rust/tonk-ui/styles.css'));
for (const match of [...css.matchAll(/url\(["']?(\/fonts\/([^"')]+))["']?\)/g)]) {
  const name = match[2];
  if (!/^[A-Za-z0-9_.-]+\.(woff2?|otf|ttf)$/.test(name)) throw Error('Invalid font asset');
  const mime = name.endsWith('.woff2')
    ? 'font/woff2'
    : name.endsWith('.woff')
      ? 'font/woff'
      : name.endsWith('.otf')
        ? 'font/otf'
        : 'font/ttf';
  const font = await readFile(new URL('../../rust/tonk-ui/assets/fonts/' + name, import.meta.url));
  css = css.replaceAll(match[0], `url("data:${mime};base64,${font.toString('base64')}")`);
}
const theme = await readFile(new URL('../apps/ui/space-theme.js', import.meta.url), 'utf8');
const pack = {
  manifest,
  glue,
  snippets,
  wasm: cached ? cached.wasm : await get('/guest/' + manifest.wasm, true),
  wa: cached ? cached.wa : await get('/guest/' + manifest.waJs, true),
  waCss,
  css,
  theme,
  bootstrap: await local('rust/tonk-site/src/bootstrap.js'),
  runtimeBootstrap: await local('rust/tonk-site/src/runtime_bootstrap.js'),
};
if (
  !Buffer.from(pack.wasm, 'base64')
    .subarray(0, 4)
    .equals(Buffer.from([0, 97, 115, 109]))
)
  throw Error('Guest Wasm URL did not return a Wasm binary');
if (snippets.some((s) => typeof s.src !== 'string' || s.src.trimStart().startsWith('<')))
  throw Error('Invalid guest snippet');
await mkdir(new URL('./public/', import.meta.url), { recursive: true });
await writeFile(new URL('./public/space-runtime.json', import.meta.url), JSON.stringify(pack));
console.log(`Built real guest runtime: ${snippets.length} snippets`);

const guest = `<!doctype html><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval' blob:; style-src 'unsafe-inline' blob:; img-src data: blob:; font-src data: blob:; connect-src blob:; frame-src blob: about:; form-action 'none'"><style>html,body{margin:0;min-height:100%;font-family:system-ui}</style>`;
const script = (source) => '<script>' + source.replace(/<\/script/gi, '<\\/script') + '</script>';
// Full worker host: no component-specific edit locks or command adapters.
const workerMount = await readFile(
  new URL('../apps/ui/embed-bootstrap.js', import.meta.url),
  'utf8',
);
await writeFile(
  new URL('./public/worker-guest.html', import.meta.url),
  guest +
    script(theme) +
    script(workerMount) +
    script(pack.bootstrap) +
    script(pack.runtimeBootstrap) +
    '<body></body>',
);
