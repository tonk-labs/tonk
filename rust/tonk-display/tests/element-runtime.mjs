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

// 14. A legacy `component` module and a table-driven `element` share
// the realm. The old shape calls `customElements.define` itself; the
// new one goes through the table. Neither knows about the other, which
// is exactly why a branch can carry both without migrating.
check('a legacy module and a table-driven element coexist', await page.evaluate(() => {
  // What `<tonk-component>` injects for a `component` row.
  const script = document.createElement('script');
  script.textContent = `customElements.get('legacy-widget') || customElements.define('legacy-widget',
    class extends HTMLElement { connectedCallback() { this.textContent = 'legacy'; } });`;
  document.head.append(script);

  defineTonkElement('modern-widget', { connected: (self) => { self.textContent = 'modern'; } });

  const legacy = document.createElement('legacy-widget');
  const modern = document.createElement('modern-widget');
  document.body.append(legacy, modern);
  return legacy.textContent === 'legacy' && modern.textContent === 'modern';
}));

// 15. Editing the modern one still live-swaps with a legacy element
// present — the table is per-tag, so the legacy registration is
// untouched.
check('editing an element leaves a legacy neighbour alone', await page.evaluate(() => {
  defineTonkElement('modern-widget', { connected: (self) => { self.textContent = 'modern v2'; } });
  return document.querySelector('modern-widget').textContent === 'modern v2'
      && document.querySelector('legacy-widget').textContent === 'legacy';
}));

// --- on-demand resolution -------------------------------------------
// These drive the REAL protocol: discovery dispatches
// `tonk-element-needed`, a document-level listener answers by calling
// `defineTonkElement`. Only the listener is stubbed (the host's runs a
// branch query in wasm) — the announcement, the bubbling, the
// de-duplication and the upgrade are the shipped code.

// 16. A tag nobody registered is announced because it rendered, and a
// document listener can register it from there.
check('an unregistered tag announces itself and gets registered', await page.evaluate(async () => {
  globalThis.asked = [];
  globalThis.mainListener = (event) => {
    const { tag } = event.detail;
    globalThis.asked.push(tag);
    // Claim it: the announcement is only remembered once someone
    // claims it, so an unclaimed tag is re-offered on the next
    // mutation. Claiming means "mine to answer", not "answered" — the
    // decline below still counts as handled.
    event.preventDefault();
    // A listener with no definition for a tag simply does not register
    // it; the element stays inert, as it was before it announced.
    if (tag.endsWith('-missing')) return;
    defineTonkElement(tag, { connected: (self) => { self.textContent = `resolved ${tag}`; } });
  };
  document.addEventListener('tonk-element-needed', globalThis.mainListener);
  startTonkElements();
  const el = document.createElement('lazy-one');
  document.body.append(el);
  await new Promise(r => setTimeout(r, 0));
  return globalThis.asked.includes('lazy-one') && el.textContent === 'resolved lazy-one';
}));

// 17. The event bubbles from the element, not from document — that is
// what lets a listener read routing context off the element's
// ancestors.
check('the event bubbles from the element that needs it', await page.evaluate(async () => {
  globalThis.seen = null;
  document.addEventListener('tonk-element-needed', (event) => {
    globalThis.seen = {
      tag: event.detail.tag,
      fromElement: event.target instanceof Element,
      targetTag: event.target.tagName.toLowerCase(),
      context: event.target.closest('[with]')?.getAttribute('with') ?? null,
    };
  }, { once: true });
  const host = document.createElement('div');
  host.setAttribute('with', 'main@repo');
  host.innerHTML = '<lazy-ctx></lazy-ctx>';
  document.body.append(host);
  await new Promise(r => setTimeout(r, 0));
  return globalThis.seen?.tag === 'lazy-ctx'
      && globalThis.seen.fromElement
      && globalThis.seen.targetTag === 'lazy-ctx'
      && globalThis.seen.context === 'main@repo';
}));

// 18. Announced once per tag, however many instances render.
check('a tag is announced once per realm', await page.evaluate(async () => {
  globalThis.asked = [];
  for (let i = 0; i < 3; i++) document.body.append(document.createElement('lazy-two'));
  await new Promise(r => setTimeout(r, 0));
  return globalThis.asked.filter(t => t === 'lazy-two').length === 1;
}));

// 19. A tag the listener declines is announced once and stays inert.
check('an unanswerable tag is announced once and stays inert', await page.evaluate(async () => {
  globalThis.asked = [];
  const el = document.createElement('lazy-missing');
  document.body.append(el);
  await new Promise(r => setTimeout(r, 0));
  document.body.append(document.createElement('lazy-missing'));
  await new Promise(r => setTimeout(r, 0));
  return globalThis.asked.filter(t => t === 'lazy-missing').length === 1
      && !customElements.get('lazy-missing');
}));

// 20. An announcement nobody claims is offered again. This is what
// keeps a tag that rendered before its listener existed from being
// inert forever — the one announcement would otherwise be the only one.
check('an unclaimed announcement is re-offered', await page.evaluate(async () => {
  globalThis.unclaimed = 0;
  const count = (event) => { if (event.detail.tag === 'lazy-unclaimed') globalThis.unclaimed++; };
  // Stand the claiming listener down so this announcement goes
  // unclaimed — the state a page is in before its registry installs.
  document.removeEventListener('tonk-element-needed', globalThis.mainListener);
  document.addEventListener('tonk-element-needed', count);
  document.body.append(document.createElement('lazy-unclaimed'));
  await new Promise(r => setTimeout(r, 0));
  const first = globalThis.unclaimed;
  // Re-offered when the tag NEXT APPEARS — the observer reports added
  // subtrees, so an element sitting unclaimed where it already is does
  // not announce again on its own.
  document.body.append(document.createElement('lazy-unclaimed'));
  await new Promise(r => setTimeout(r, 0));
  document.removeEventListener('tonk-element-needed', count);
  document.addEventListener('tonk-element-needed', globalThis.mainListener);
  return first === 1 && globalThis.unclaimed > 1;
}));

// 21. Built-in and vendor prefixes are left to their own loaders.
check('tonk- and wa- tags are not claimed', await page.evaluate(async () => {
  globalThis.asked = [];
  document.body.append(document.createElement('tonk-whatever'));
  document.body.append(document.createElement('wa-whatever'));
  await new Promise(r => setTimeout(r, 0));
  return globalThis.asked.length === 0;
}));

// 22. A tag deep in an added subtree is found — a view renders a
// fragment, not one element at a time.
check('a tag deep in an added subtree is announced', await page.evaluate(async () => {
  globalThis.asked = [];
  const wrapper = document.createElement('div');
  wrapper.innerHTML = '<section><p><lazy-deep></lazy-deep></p></section>';
  document.body.append(wrapper);
  await new Promise(r => setTimeout(r, 0));
  return globalThis.asked.includes('lazy-deep')
      && wrapper.querySelector('lazy-deep').textContent === 'resolved lazy-deep';
}));

// 23. Somewhere the observer cannot see — a shadow root — can announce
// its own tags by hand, and the same listener services them.
check('a shadow root can announce its own tags', await page.evaluate(async () => {
  globalThis.asked = [];
  const host = document.createElement('div');
  document.body.append(host);
  const root = host.attachShadow({ mode: 'open' });
  root.innerHTML = '<lazy-shadow></lazy-shadow>';
  await new Promise(r => setTimeout(r, 0));
  const unseenByObserver = !globalThis.asked.includes('lazy-shadow');
  announceTonkElements(root);
  await new Promise(r => setTimeout(r, 0));
  return unseenByObserver
      && globalThis.asked.includes('lazy-shadow')
      && root.querySelector('lazy-shadow').textContent === 'resolved lazy-shadow';
}));

// --- swap hooks ------------------------------------------------------

// 24. The outgoing definition tears down before the incoming takes
// over. Without `released`, re-running `connected` would stack setup on
// top of setup — a listener per edit, with no way to undo it.
check('released runs on the outgoing definition before the swap', await page.evaluate(async () => {
  globalThis.order = [];
  defineTonkElement('swap-el', {
    connected: (self) => { globalThis.order.push('connected-v1'); self.textContent = 'v1'; },
    released: (self) => { globalThis.order.push('released-v1'); },
  });
  const el = document.createElement('swap-el');
  document.body.append(el);

  defineTonkElement('swap-el', {
    connected: (self) => { globalThis.order.push('connected-v2'); self.textContent = 'v2'; },
  });
  return JSON.stringify(globalThis.order) ===
      JSON.stringify(['connected-v1', 'released-v1', 'connected-v2'])
    && el.textContent === 'v2';
}));

// 25. `released` is called through the OUTGOING function, not the table
// — the table already holds the replacement by then.
check('released is the outgoing implementation, not the new one', await page.evaluate(async () => {
  globalThis.which = null;
  defineTonkElement('which-el', {
    connected: () => {},
    released: () => { globalThis.which = 'old'; },
  });
  document.body.append(document.createElement('which-el'));
  defineTonkElement('which-el', {
    connected: () => {},
    released: () => { globalThis.which = 'new'; },
  });
  return globalThis.which === 'old';
}));

// 26. A definition that declares `swapped` takes over live instances
// itself; `connected` stays for a fresh mount.
check('swapped takes over instead of re-running connected', await page.evaluate(async () => {
  globalThis.calls = [];
  defineTonkElement('taken-over', {
    connected: (self) => { globalThis.calls.push('connected'); self.textContent = 'first'; },
  });
  const live = document.createElement('taken-over');
  document.body.append(live);

  defineTonkElement('taken-over', {
    connected: (self) => { globalThis.calls.push('connected'); self.textContent = 'fresh'; },
    swapped: (self) => { globalThis.calls.push('swapped'); self.textContent = 'migrated'; },
  });
  const migrated = live.textContent === 'migrated';

  // A NEW instance still goes through `connected`.
  const fresh = document.createElement('taken-over');
  document.body.append(fresh);
  return migrated
    && fresh.textContent === 'fresh'
    && JSON.stringify(globalThis.calls) === JSON.stringify(['connected', 'swapped', 'connected']);
}));

// 27. State handed from one implementation to the next, which is what
// the pair is for.
check('released can hand state to swapped', await page.evaluate(async () => {
  defineTonkElement('handover-el', {
    connected: (self) => { self.dataset.count = '7'; },
    released: (self) => { self.dataset.carried = self.dataset.count; },
  });
  const el = document.createElement('handover-el');
  document.body.append(el);
  defineTonkElement('handover-el', {
    connected: (self) => { self.textContent = 'fresh'; },
    swapped: (self) => { self.textContent = `kept ${self.dataset.carried}`; },
  });
  return el.textContent === 'kept 7';
}));

// 28. The real reason `released` exists: teardown. A listener added on
// connect and removed on release must not fire twice after an edit.
check('released prevents setup stacking across edits', await page.evaluate(async () => {
  globalThis.fired = 0;
  const bump = () => { globalThis.fired++; };
  defineTonkElement('listen-el', {
    connected: (self) => { self.__bump = bump; self.addEventListener('ping', self.__bump); },
    released: (self) => { self.removeEventListener('ping', self.__bump); },
  });
  const el = document.createElement('listen-el');
  document.body.append(el);
  defineTonkElement('listen-el', {
    connected: (self) => { self.__bump = bump; self.addEventListener('ping', self.__bump); },
    released: (self) => { self.removeEventListener('ping', self.__bump); },
  });
  el.dispatchEvent(new CustomEvent('ping'));
  return globalThis.fired === 1;
}));

// 29. Swap hooks fire on ANY change to the definition, not only when
// `connected` moved — adding a method is a swap too. But `connected`
// itself is only re-run when it actually changed.
check('a changed method fires the hooks without re-running connected', await page.evaluate(async () => {
  globalThis.log = [];
  const connected = (self) => { globalThis.log.push('connected'); };
  defineTonkElement('grow-el', {
    connected,
    released: () => { globalThis.log.push('released'); },
  });
  document.body.append(document.createElement('grow-el'));
  // Same `connected`, one method added.
  defineTonkElement('grow-el', {
    connected,
    released: () => { globalThis.log.push('released'); },
    extra: () => 1,
  });
  return JSON.stringify(globalThis.log) === JSON.stringify(['connected', 'released']);
}));

// 30. An identical redefinition is not a swap at all.
check('an unchanged definition fires no swap hooks', await page.evaluate(async () => {
  globalThis.quiet = 0;
  const methods = {
    connected: () => {},
    released: () => { globalThis.quiet++; },
  };
  defineTonkElement('quiet-el', methods);
  document.body.append(document.createElement('quiet-el'));
  defineTonkElement('quiet-el', { ...methods });
  return globalThis.quiet === 0;
}));

// 31. Ordinary removal is `disconnected`, not `released` — the two say
// different things and must not be conflated.
check('removing an element calls disconnected, not released', await page.evaluate(async () => {
  globalThis.events = [];
  defineTonkElement('bye2-el', {
    connected: () => {},
    disconnected: () => { globalThis.events.push('disconnected'); },
    released: () => { globalThis.events.push('released'); },
  });
  const el = document.createElement('bye2-el');
  document.body.append(el);
  el.remove();
  return JSON.stringify(globalThis.events) === JSON.stringify(['disconnected']);
}));

await browser.close();
let failed = 0;
for (const r of results) {
  console.log(`${r.ok ? 'ok  ' : 'FAIL'}  ${r.name}${r.detail ? ' — ' + r.detail : ''}`);
  if (!r.ok) failed++;
}
console.log(`\n${results.length - failed}/${results.length} passed`);
process.exit(failed ? 1 : 0);
