// Behaviour tests for `assets/element-runtime.js`, driven in a real
// Chromium.
//
// The runtime is plain JS on purpose: the wasm side only folds facts
// and generates `defineTonkElement(tag, methods)` calls, so everything
// that touches `customElements` and the DOM lifecycle is testable
// without a wasm toolchain. That matters because the properties worth
// proving here — that an edited method reaches instances already
// mounted, and that the tag is registered exactly once — are the whole
// reason the runtime dispatches through a table instead of baking the
// author's functions into the class.
//
// Run it:
//
//   node rust/tonk-display/tests/element-runtime.mjs
//
// It resolves Playwright and Chromium from the paths the dev image
// provides, overridable with PLAYWRIGHT_ROOT / CHROMIUM_BIN.

import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const PLAYWRIGHT_ROOT =
  process.env.PLAYWRIGHT_ROOT ?? '/opt/node22/lib/node_modules/playwright';
const CHROMIUM_BIN =
  process.env.CHROMIUM_BIN ?? '/opt/pw-browsers/chromium-1194/chrome-linux/chrome';
const { chromium } = await import(join(PLAYWRIGHT_ROOT, 'index.mjs'));

const here = dirname(fileURLToPath(import.meta.url));
const RUNTIME = readFileSync(join(here, '..', 'assets', 'element-runtime.js'), 'utf8');
const results = [];
const check = (name, ok, detail = '') =>
  results.push({ name, ok, detail: ok ? '' : detail });

const browser = await chromium.launch({
  executablePath: CHROMIUM_BIN,
  // The page needs no network; going through a proxy only adds
  // startup probes that can stall a sandboxed run.
  args: ['--no-proxy-server', '--disable-background-networking'],
});
const page = await browser.newPage();
page.on('console', m => { if (m.type() === 'error' || m.type() === 'warning') console.log(`  [console.${m.type()}] ${m.text()}`); });
await page.setContent('<!doctype html><html><body></body></html>');
await page.addScriptTag({ content: RUNTIME });

// 1. lifecycle: connected fires and can mutate the element
check('connected fires on mount', await page.evaluate(() => {
  defineTonkElement('tally-widget', {
    connected: (self) => { self.textContent = `count: ${self.getAttribute('count') ?? 0}`; },
  });
  const el = document.createElement('tally-widget');
  el.setAttribute('count', '3');
  document.body.append(el);
  return el.textContent === 'count: 3';
}));

// 2. LIVE SWAP: edit connected, already-mounted instance re-runs
check('editing connected re-runs it on live instances', await page.evaluate(() => {
  defineTonkElement('tally-widget', {
    connected: (self) => { self.textContent = `TOTAL ${self.getAttribute('count')}`; },
  });
  return document.querySelector('tally-widget').textContent === 'TOTAL 3';
}));

// 3. the tag was never re-registered
check('customElements.define called once', await page.evaluate(() => {
  const ctor = customElements.get('tally-widget');
  defineTonkElement('tally-widget', { connected: (s) => { s.textContent = 'again'; } });
  return customElements.get('tally-widget') === ctor;
}));

// 4. custom method lands camelCased and dispatches live
check('custom method is callable and live', await page.evaluate(() => {
  defineTonkElement('bump-btn', { 'do-bump': (self) => 'v1' });
  const el = document.createElement('bump-btn');
  document.body.append(el);
  const first = el.doBump();
  defineTonkElement('bump-btn', { 'do-bump': (self) => 'v2' });
  return first === 'v1' && el.doBump() === 'v2';
}));

// 5. attribute-changed via MutationObserver, incl. replay at upgrade
check('attribute-changed replays initial then observes', await page.evaluate(async () => {
  globalThis.seen = [];
  defineTonkElement('attr-el', {
    'attribute-changed': (self, name, before, after) => { globalThis.seen.push([name, before, after]); },
  });
  const el = document.createElement('attr-el');
  el.setAttribute('a', '1');
  document.body.append(el);
  const replayed = JSON.stringify(globalThis.seen) === JSON.stringify([['a', null, '1']]);
  el.setAttribute('a', '2');
  await new Promise(r => setTimeout(r, 0));
  return replayed && JSON.stringify(globalThis.seen.at(-1)) === JSON.stringify(['a', '1', '2']);
}));

// 6. disconnected fires and the instance leaves the live set
check('disconnected fires and drops the instance', await page.evaluate(() => {
  let gone = false;
  defineTonkElement('bye-el', { connected: () => {}, disconnected: () => { gone = true; } });
  const el = document.createElement('bye-el');
  document.body.append(el);
  const sizeWhileLive = __tonkElements.live.get('bye-el').size;
  el.remove();
  return gone && sizeWhileLive === 1 && __tonkElements.live.get('bye-el').size === 0;
}));

// 7. a throwing method does not break the page or siblings
check('a throwing method is contained', await page.evaluate(() => {
  defineTonkElement('bad-el', { connected: () => { throw new Error('boom'); } });
  const bad = document.createElement('bad-el');
  document.body.append(bad);
  const sibling = document.createElement('tally-widget');
  document.body.append(sibling);
  return sibling.textContent === 'again';
}));

// 8. a method shadowing an HTMLElement member is refused
check('shadowing method is refused', await page.evaluate(() => {
  const before = HTMLElement.prototype.remove;
  defineTonkElement('shadow-el', { remove: (self) => 'hijacked' });
  const el = document.createElement('shadow-el');
  document.body.append(el);
  const ok = el.remove !== 'hijacked' && typeof el.remove === 'function';
  el.remove();
  return ok && HTMLElement.prototype.remove === before && !el.isConnected;
}));

// 9. the `define` escape hatch registers the returned class
check('define key registers a raw class', await page.evaluate(() => {
  defineTonkElement('raw-el', {
    define: () => class extends HTMLElement {
      connectedCallback() { this.textContent = 'raw'; }
      static get observedAttributes() { return ['x']; }
    },
  });
  const el = document.createElement('raw-el');
  document.body.append(el);
  return el.textContent === 'raw' && customElements.get('raw-el').observedAttributes[0] === 'x';
}));

// 10. an element already defined elsewhere is left alone
check('a pre-existing definition is not clobbered', await page.evaluate(() => {
  customElements.define('taken-el', class extends HTMLElement {
    connectedCallback() { this.textContent = 'original'; }
  });
  defineTonkElement('taken-el', { connected: (s) => { s.textContent = 'branch'; } });
  const el = document.createElement('taken-el');
  document.body.append(el);
  return el.textContent === 'original';
}));

// 11. elements already in the DOM upgrade when the tag is defined later
check('a later definition upgrades elements already rendered', await page.evaluate(() => {
  const el = document.createElement('late-el');
  document.body.append(el);
  const inertBefore = el.textContent === '';
  defineTonkElement('late-el', { connected: (s) => { s.textContent = 'upgraded'; } });
  return inertBefore && el.textContent === 'upgraded';
}));

// 12. A hook that WRITES an attribute must not re-enter itself. This
// is the hazard that comes with observing every attribute instead of a
// declared `observedAttributes` list: without draining the observer
// after each call, this handler loops until the renderer dies.
check('an attribute-writing hook does not re-enter forever', await page.evaluate(async () => {
  defineTonkElement('mirror-el', {
    'attribute-changed': (self, name, before, after) => { self.dataset.last = `${name}:${after}`; },
  });
  const el = document.createElement('mirror-el');
  el.setAttribute('count', '7');
  document.body.append(el);
  await new Promise(r => setTimeout(r, 0));
  el.setAttribute('count', '9');
  await new Promise(r => setTimeout(r, 0));
  return el.dataset.last === 'count:9';
}));

// 13. A no-op write reports nothing: the observer sees a WRITE, the
// hook should see a CHANGE.
check('rewriting the same value fires nothing', await page.evaluate(async () => {
  globalThis.hits = 0;
  defineTonkElement('noop-el', { 'attribute-changed': () => { globalThis.hits++; } });
  const el = document.createElement('noop-el');
  el.setAttribute('a', '1');
  document.body.append(el);
  await new Promise(r => setTimeout(r, 0));
  const afterReplay = globalThis.hits;
  el.setAttribute('a', '1');
  await new Promise(r => setTimeout(r, 0));
  return afterReplay === 1 && globalThis.hits === 1;
}));

await browser.close();
let failed = 0;
for (const r of results) {
  console.log(`${r.ok ? 'ok  ' : 'FAIL'}  ${r.name}${r.detail ? ' — ' + r.detail : ''}`);
  if (!r.ok) failed++;
}
console.log(`\n${results.length - failed}/${results.length} passed`);
process.exit(failed ? 1 : 0);
