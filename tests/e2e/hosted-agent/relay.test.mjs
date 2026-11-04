// The relay page alone, in a real browser, as an MCP Apps host shows it:
// Tonk's line under the view, loading and failure, display modes, links
// and theme. The provider is a stand-in (lib/relay-stand-in.mjs), so no
// deployment, account or space is needed:
//
//   npm run test:relay
//
// What it cannot show: a space rendering, a real host's policies, and how
// Claude or ChatGPT lay a card out.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { showView } from './lib/view-host.mjs';
import { relayResource, standIn, standInClient } from './lib/relay-stand-in.mjs';

const SPACE = 'did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK';

// What `tonk_open_space` answers for one view of Family trip.
const result = (extra = {}) => ({
  content: [{ type: 'text', text: 'Showing that view inline.' }],
  structuredContent: {
    subject: SPACE,
    path: '/',
    view: { model: 'tonk/list', entity: 'tonk:packing', facet: null },
    edit: true,
    remote: 'http://127.0.0.1/ucan/',
    name: 'Family trip',
    viewName: 'Packing list',
    role: 'write',
    ...extra,
  },
});

const granted = {
  isError: false,
  structuredContent: { delegations: ['stand-in'], expiresAt: Date.now() + 3_600_000 },
};

// A peer that pulls, answers the page's site, and says it is synced.
const PEER = {
  '/sync/pull': { body: {} },
  '/site': { body: { site: 'urn:stand-in:site' } },
  '/sync/status': { body: { state: 'synced' } },
};

// A guest with a bridge: it opens the bridge, then reports a size.
const SPEAKS = [{ hello: true }, { wait: 300 }, { type: 'tonk-view-size', height: 180 }];

async function shown({ peer = PEER, guest = SPEAKS, access = granted, extra, context } = {}) {
  const provider = await standIn({ peer, guest });
  const client = standInClient(access);
  const view = await showView({
    client,
    result: result({ remote: `${provider.origin}/ucan/`, ...extra }),
    resource: await relayResource(provider.origin),
    frames: false,
    shared: false,
    context,
  });
  const relay = async () => {
    for (let tries = 0; tries < 50; tries++) {
      const frame = view.relay();
      if (frame && (await frame.evaluate(() => !!document.querySelector('#line')).catch(() => false)))
        return frame;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    throw new Error('the relay never loaded');
  };
  return {
    view,
    provider,
    client,
    relay,
    async close() {
      await view.close();
      await provider.close();
    },
  };
}

// Wait until `check` (run in the relay) answers truthy.
async function until(frame, check, arg, timeout = 15_000) {
  await frame.waitForFunction(check, arg, { timeout, polling: 100 });
}

const lineOf = (frame) =>
  frame.evaluate(() => {
    const line = document.querySelector('#line');
    const disc = line.querySelector('.tonk-disc');
    return {
      space: line.querySelector('.tonk-space').textContent,
      where: line.querySelector('.tonk-where').textContent,
      role: line.querySelector('.tonk-role').textContent,
      disc: disc.dataset.state ?? 'filled',
      discLabel: disc.getAttribute('aria-label'),
      discRole: disc.getAttribute('role'),
      lineHeight: line.getBoundingClientRect().height,
      font: getComputedStyle(line).fontFamily,
    };
  });

test("Tonk's line names the space, the view and the role under the view", async () => {
  const card = await shown();
  try {
    const frame = await card.relay();
    const line = await lineOf(frame);
    assert.equal(line.space, 'Family trip');
    assert.equal(line.where, '› Packing list');
    assert.equal(line.role, 'write');
    assert.equal(line.discRole, 'img');
    assert.equal(line.lineHeight, 30, 'the line is 30px');
    assert.match(line.font, /IBM Plex Sans Condensed/);
    const order = await frame.evaluate(() => {
      const card = document.querySelector('#card');
      return [...card.children].map((child) => child.className || child.id);
    });
    assert.equal(order[0], 'tonk-well', 'the view comes first');
    assert.equal(order.at(-1), 'tonk-line', 'the line is under it');
    // The fonts come from the provider, with CORS.
    await until(frame, () => document.fonts.check('600 14px "IBM Plex Sans Condensed"'));
    assert.ok(card.provider.served.some((path) => path.startsWith('/in-agent/fonts/')));
    assert.deepEqual(card.view.violations, []);
  } finally {
    await card.close();
  }
});

test('a read-only connection says read, and an unnamed view names only its space', async () => {
  const card = await shown({ extra: { role: 'read', edit: false, viewName: null } });
  try {
    const line = await lineOf(await card.relay());
    assert.equal(line.role, 'read');
    assert.equal(line.space, 'Family trip');
    assert.equal(line.where, '');
  } finally {
    await card.close();
  }
});

test('the disc is hollow until the space is pulled, then filled', async () => {
  const card = await shown({ guest: [{ hello: true }] });
  try {
    const frame = await card.relay();
    await until(frame, () => !document.querySelector('#line .tonk-disc').dataset.state);
    const line = await lineOf(frame);
    assert.equal(line.disc, 'filled');
    assert.equal(line.discLabel, 'synced');
  } finally {
    await card.close();
  }
});

test('the disc goes hollow when the space cannot be reached', async () => {
  const card = await shown({
    peer: { ...PEER, '/sync/status': { status: 503, body: { error: 'SYNC_UNAVAILABLE' } } },
  });
  try {
    const frame = await card.relay();
    await until(frame, () => !document.querySelector('#line .tonk-disc').dataset.state);
    await card.view.page.context().setOffline(true);
    await until(frame, () => document.querySelector('#line .tonk-disc').dataset.state === 'hollow');
    assert.equal((await lineOf(frame)).discLabel, 'local only');
  } finally {
    await card.close();
  }
});

test('the well is busy until the view reports its size, and says what is loading', async () => {
  // A page with a bridge that takes its time to report a size.
  const card = await shown({ guest: [{ hello: true }, { wait: 2500 }, { type: 'tonk-view-size', height: 220 }] });
  try {
    const frame = await card.relay();
    await until(frame, () => document.querySelector('.tonk-well').dataset.loading);
    const busy = await frame.evaluate(() => {
      const well = document.querySelector('.tonk-well');
      return { busy: well.getAttribute('aria-busy'), says: well.dataset.loading };
    });
    assert.deepEqual(busy, { busy: 'true', says: 'loading Packing list' });
    const label = () => frame.evaluate(() => document.querySelector('#card').getAttribute('aria-label'));
    assert.equal(await label(), 'Tonk: Family trip, Packing list, loading');
    await until(frame, () => !document.querySelector('.tonk-well').hasAttribute('aria-busy'));
    const height = await frame.evaluate(() => document.querySelector('.tonk-view').offsetHeight);
    assert.equal(height, 220);
    assert.equal(await label(), 'Tonk: Family trip, Packing list');
    // The host is told the whole card: the view and the line under it.
    assert.equal(await card.view.height(), 250);
  } finally {
    await card.close();
  }
});

test('a page that loads and stays silent fails after 3 s, and try again loads it once more', async () => {
  const card = await shown({ guest: null });
  try {
    const frame = await card.relay();
    await until(frame, () => document.querySelector('.tonk-well').hasAttribute('data-failed'), null, 10_000);
    const failed = await frame.evaluate(() => ({
      says: document.querySelector('.tonk-failed').textContent,
      role: document.querySelector('.tonk-failed').getAttribute('role'),
      busy: document.querySelector('.tonk-well').getAttribute('aria-busy'),
    }));
    assert.equal(failed.says, 'Packing list didn’t load. try again');
    assert.equal(failed.role, 'alert');
    assert.equal(failed.busy, null, 'it no longer says loading');
    assert.equal(
      await frame.evaluate(() => document.querySelector('#card').getAttribute('aria-label')),
      'Tonk: Family trip, Packing list, didn’t load',
    );
    const pulls = card.provider.served.filter((path) => path === '/worker-guest').length;
    await frame.click('.tonk-failed .tonk-link');
    await until(frame, () => !document.querySelector('.tonk-failed'));
    await until(frame, () => document.querySelector('.tonk-well').hasAttribute('data-failed'), null, 10_000);
    assert.ok(
      card.provider.served.filter((path) => path === '/worker-guest').length > pulls,
      'try again loads the page again',
    );
  } finally {
    await card.close();
  }
});

test('a slow page that speaks is never marked failed', async () => {
  // A cold load: the bridge answers at once, the size only after 5 s.
  const card = await shown({ guest: [{ hello: true }, { wait: 5000 }, { type: 'tonk-view-size', height: 160 }] });
  try {
    const frame = await card.relay();
    await until(frame, () => !document.querySelector('.tonk-well').hasAttribute('aria-busy'), null, 12_000);
    assert.equal(await frame.evaluate(() => document.querySelector('.tonk-failed')), null);
  } finally {
    await card.close();
  }
});

test('access the space refuses fails the view with its reason', async () => {
  const card = await shown({
    access: { isError: true, content: [{ type: 'text', text: 'This connection does not reach that space.' }] },
  });
  try {
    const frame = await card.relay();
    await until(frame, () => document.querySelector('.tonk-well').hasAttribute('data-failed'));
    const said = await frame.evaluate(() => document.querySelector('#error').textContent);
    assert.equal(said, 'This connection does not reach that space.');
    assert.equal((await lineOf(frame)).disc, 'hollow', 'nothing synced');
  } finally {
    await card.close();
  }
});

// The stand-in guest page, once it has loaded.
async function guestOf(card) {
  for (let tries = 0; tries < 100; tries++) {
    for (const frame of card.view.page.frames()) {
      const found = await frame
        .evaluate(() => document.body?.textContent.includes('stand-in guest') && Array.isArray(window.heard))
        .catch(() => false);
      if (found) return frame;
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error('the guest page never loaded');
}

test('the full-screen button asks the agent app, and full screen fills the card and opens Tonk', async () => {
  const card = await shown({ context: { safeAreaInsets: { top: 0, right: 0, bottom: 24, left: 0 } } });
  try {
    const frame = await card.relay();
    await until(frame, () => !document.querySelector('#expand').hidden);
    assert.equal(await frame.evaluate(() => document.querySelector('#out').hidden), true);
    await frame.click('#expand');
    await until(frame, () => document.querySelector('#card').classList.contains('tonk--fill'));
    const asked = await card.view.asked();
    assert.deepEqual(
      asked.filter((ask) => ask.method === 'ui/request-display-mode'),
      [{ method: 'ui/request-display-mode', mode: 'fullscreen' }],
    );
    const fill = await frame.evaluate(() => ({
      expand: document.querySelector('#expand').hidden,
      out: document.querySelector('#out').hidden,
      card: document.querySelector('#card').getBoundingClientRect().height,
      viewport: innerHeight,
      bottom: getComputedStyle(document.querySelector('#line')).paddingBottom,
    }));
    assert.equal(fill.expand, true, 'no full-screen button in full screen');
    assert.equal(fill.out, false, 'open in tonk shows');
    assert.equal(fill.card, fill.viewport, 'the card fills the agent app’s sheet');
    assert.equal(fill.bottom, '24px', 'the line clears the chat input');
    await until(frame, () => document.querySelector('#card').getAttribute('aria-label').endsWith(', full screen'));
    assert.equal(
      await frame.evaluate(() => document.querySelector('#out').textContent),
      'open in tonk ↗',
    );
    await frame.click('#out');
    await until(frame, () => true);
    const opened = (await card.view.asked()).filter((ask) => ask.method === 'ui/open-link');
    assert.deepEqual(opened, [
      { method: 'ui/open-link', url: `${card.provider.origin}/space/${SPACE}` },
    ]);
    // Back inline, as the agent app closes its sheet.
    await card.view.changeContext({ displayMode: 'inline' });
    await until(frame, () => !document.querySelector('#card').classList.contains('tonk--fill'));
  } finally {
    await card.close();
  }
});

test('an agent app with no full screen gets no full-screen button', async () => {
  const card = await shown({ context: { availableDisplayModes: ['inline'] } });
  try {
    const frame = await card.relay();
    await until(frame, () => document.querySelector('#line .tonk-space').textContent);
    assert.equal(await frame.evaluate(() => document.querySelector('#expand').hidden), true);
  } finally {
    await card.close();
  }
});

test('docked, the card fills its frame and does not offer Tonk', async () => {
  const card = await shown({ context: { displayMode: 'pip', availableDisplayModes: ['inline', 'pip', 'fullscreen'] } });
  try {
    const frame = await card.relay();
    await until(frame, () => document.querySelector('#card').classList.contains('tonk--fill'));
    assert.equal(await frame.evaluate(() => document.querySelector('#out').hidden), true);
  } finally {
    await card.close();
  }
});

test("a link out of the view opens through the agent app's open-link", async () => {
  const card = await shown({
    guest: [
      { hello: true },
      { type: 'tonk-view-size', height: 120 },
      { wait: 300 },
      { port: { v: 1, type: 'navigate', href: 'https://example.com/a?b=c' } },
      // What the stock guest sends for an external or new-tab link.
      { port: { v: 1, type: 'open', href: 'https://example.org/doc' } },
      { port: { v: 1, type: 'open', href: '/packing?tab=2' } },
    ],
  });
  try {
    const frame = await card.relay();
    await until(frame, () => !document.querySelector('.tonk-well').hasAttribute('aria-busy'));
    for (let tries = 0; tries < 50; tries++) {
      const asked = await card.view.asked();
      if (asked.filter((ask) => ask.method === 'ui/open-link').length >= 3) break;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    const opened = (await card.view.asked()).filter((ask) => ask.method === 'ui/open-link');
    assert.deepEqual(opened, [
      { method: 'ui/open-link', url: 'https://example.com/a?b=c' },
      { method: 'ui/open-link', url: 'https://example.org/doc' },
      { method: 'ui/open-link', url: `${card.provider.origin}/packing?tab=2` },
    ]);
    assert.equal(await frame.evaluate(() => document.querySelector('#error').hidden), true);
  } finally {
    await card.close();
  }
});

test("Tonk's line and the view follow the agent app's theme", async () => {
  const card = await shown({
    context: { theme: 'dark' },
    guest: [...SPEAKS, { __tonkRuntime: 'runtime-ready' }],
  });
  try {
    const frame = await card.relay();
    await until(frame, () => document.documentElement.dataset.theme === 'dark');
    const sheet = await frame.evaluate(() => getComputedStyle(document.querySelector('#line')).color);
    assert.equal(sheet, 'rgb(247, 246, 245)', 'paper text on the dark sheet');
    const guest = await guestOf(card);
    await until(guest, () => window.heard.some((m) => m?.type === 'tonk-mount'));
    const mounted = await guest.evaluate(() => window.heard.find((m) => m?.type === 'tonk-mount'));
    assert.equal(mounted.theme, 'dark');
    await card.view.changeContext({ theme: 'light' });
    await until(frame, () => document.documentElement.dataset.theme === 'light');
    await until(guest, () => window.heard.some((m) => m?.type === 'tonk-theme' && m.theme === 'light'));
  } finally {
    await card.close();
  }
});

test('the guest page takes the theme its host passes, over the system preference', async () => {
  const { chromium } = await import('playwright');
  const { readFile } = await import('node:fs/promises');
  const ui = new URL('../../../mcp/apps/ui/', import.meta.url);
  const scripts = await Promise.all(
    ['space-theme.js', 'embed-bootstrap.js'].map((name) => readFile(new URL(name, ui), 'utf8')),
  );
  const browser = await chromium.launch({ channel: process.env.TONK_E2E_CHANNEL || undefined });
  try {
    const page = await browser.newPage({ colorScheme: 'light' });
    // The guest as a host frames it: the host is the parent.
    await page.setContent(
      `<iframe srcdoc="${`<script>${scripts.join('</script><script>')}</script>`.replaceAll('"', '&quot;')}"></iframe>`,
    );
    const guest = page.frames()[1];
    await guest.waitForFunction(() => typeof window.__tonkSetTheme === 'function');
    const classes = () => guest.evaluate(() => document.documentElement.className);
    assert.match(await classes(), /wa-light/);
    await page.evaluate(() =>
      document.querySelector('iframe').contentWindow.postMessage(
        { type: 'tonk-mount', subject: 'did:key:z', site: 'urn:s', model: 'tonk:site', theme: 'dark' },
        '*',
      ),
    );
    await guest.waitForFunction(() => document.documentElement.classList.contains('wa-dark'));
    await page.emulateMedia({ colorScheme: 'dark' });
    await page.emulateMedia({ colorScheme: 'light' });
    await new Promise((resolve) => setTimeout(resolve, 200));
    assert.match(await classes(), /wa-dark/, 'the system preference no longer applies');
    // The runtime, injected after mount, sets the system's theme again.
    await guest.evaluate(() => {
      document.documentElement.classList.toggle('wa-dark', false);
      document.documentElement.classList.toggle('wa-light', true);
    });
    await guest.waitForFunction(() => document.documentElement.classList.contains('wa-dark'));
    assert.doesNotMatch(await classes(), /wa-light/, 'the host’s theme is kept');
    await page.evaluate(() =>
      document.querySelector('iframe').contentWindow.postMessage({ type: 'tonk-theme', theme: 'light' }, '*'),
    );
    await guest.waitForFunction(() => document.documentElement.classList.contains('wa-light'));
  } finally {
    await browser.close();
  }
});

test('the whole space is a card that opens it in Tonk, with no view and no peer', async () => {
  const link = 'https://tonk.example/space/' + SPACE;
  const card = await shown({ extra: { view: null, viewName: null, show: 'handoff', link } });
  try {
    const frame = await card.relay();
    await until(frame, () => !document.querySelector('#handoff').hidden);
    const drawn = await frame.evaluate(() => ({
      card: document.querySelector('#card').hidden,
      space: document.querySelector('#handoff .tonk-space').textContent,
      role: document.querySelector('#handoff .tonk-role').textContent,
      says: document.querySelector('#handoff .tonk-handoff p').textContent,
      disc: document.querySelector('#handoff .tonk-disc')?.getAttribute('aria-label'),
      arrow: document.querySelector('#handoff .tonk-btn').innerHTML,
    }));
    assert.deepEqual(drawn, {
      card: true,
      space: 'Family trip',
      role: 'write',
      says: 'Opens the whole space in Tonk.',
      disc: 'synced',
      arrow: 'open in tonk ↗',
    });
    await frame.click('#handoff .tonk-btn');
    await until(frame, () => true);
    const opened = (await card.view.asked()).filter((ask) => ask.method === 'ui/open-link');
    assert.deepEqual(opened, [{ method: 'ui/open-link', url: link }]);
    assert.ok(!card.provider.served.some((path) => path.startsWith('/view-peer/')), 'no peer starts');
    assert.equal(card.client.called.length, 0, 'no access is asked for');
  } finally {
    await card.close();
  }
});

test('a strip is one line with a link that opens the view in the browser', async () => {
  const link = `https://tonk.example/space/${SPACE}/packing`;
  const card = await shown({ extra: { show: 'strip', link } });
  try {
    const frame = await card.relay();
    await until(frame, () => !document.querySelector('#strip').hidden);
    const says = await frame.evaluate(() => document.querySelector('#strip .tonk-receipt').textContent);
    assert.equal(says, 'Family trip › Packing list opens in your browser');
    assert.equal(
      await frame.evaluate(() => document.querySelector('#strip .tonk-disc').getAttribute('role')),
      'img',
    );
    await frame.click('#strip .tonk-link');
    await until(frame, () => true);
    const opened = (await card.view.asked()).filter((ask) => ask.method === 'ui/open-link');
    assert.deepEqual(opened, [{ method: 'ui/open-link', url: link }]);
    assert.ok(!card.provider.served.some((path) => path.startsWith('/view-peer/')), 'no peer starts');
  } finally {
    await card.close();
  }
});

test('a whole view asks for full screen as it is shown, and the view hears its size', async () => {
  const card = await shown({
    extra: { size: 'full' },
    guest: [...SPEAKS, { __tonkRuntime: 'runtime-ready' }],
  });
  try {
    const frame = await card.relay();
    await until(frame, () => document.querySelector('#card').classList.contains('tonk--fill'));
    const asked = (await card.view.asked()).filter((ask) => ask.method === 'ui/request-display-mode');
    assert.deepEqual(asked, [{ method: 'ui/request-display-mode', mode: 'fullscreen' }]);
    const guest = await guestOf(card);
    await until(guest, () => window.heard.some((m) => m?.type === 'tonk-mount'));
    assert.equal(await guest.evaluate(() => window.heard.find((m) => m?.type === 'tonk-mount').size), 'full');
  } finally {
    await card.close();
  }
});

test('an inline carousel tells the view its size, and full screen tells it full', async () => {
  const card = await shown({
    extra: { size: 'carousel' },
    guest: [...SPEAKS, { __tonkRuntime: 'runtime-ready' }],
  });
  try {
    const frame = await card.relay();
    const guest = await guestOf(card);
    await until(guest, () => window.heard.some((m) => m?.type === 'tonk-mount'));
    assert.equal(await guest.evaluate(() => window.heard.find((m) => m?.type === 'tonk-mount').size), 'carousel');
    await frame.click('#expand');
    await until(guest, () => window.heard.some((m) => m?.type === 'tonk-size' && m.size === 'full'));
  } finally {
    await card.close();
  }
});

test('inline, a view drops the heading it marks; full screen it keeps it', async () => {
  const { chromium } = await import('playwright');
  const { readFile } = await import('node:fs/promises');
  const ui = new URL('../../../mcp/apps/ui/', import.meta.url);
  const scripts = await Promise.all(
    ['space-theme.js', 'embed-bootstrap.js'].map((name) => readFile(new URL(name, ui), 'utf8')),
  );
  const browser = await chromium.launch({ channel: process.env.TONK_E2E_CHANNEL || undefined });
  try {
    const page = await browser.newPage();
    await page.setContent(
      `<iframe srcdoc="${`<script>${scripts.join('</script><script>')}</script>`.replaceAll('"', '&quot;')}"></iframe>`,
    );
    const guest = page.frames()[1];
    await guest.waitForFunction(() => typeof window.__tonkSetTheme === 'function');
    const post = (message) =>
      page.evaluate((m) => document.querySelector('iframe').contentWindow.postMessage(m, '*'), message);
    await post({ type: 'tonk-mount', subject: 'did:key:z', site: 'urn:s', model: 'tonk:site', size: 'card' });
    await guest.waitForFunction(() => document.documentElement.dataset.tonkSize === 'card');
    // A view's own markup, as a template renders it into the page.
    await guest.evaluate(() => {
      const heading = document.createElement('h2');
      heading.dataset.tonkHeading = '';
      heading.textContent = 'Packing list';
      document.querySelector('tonk-display').append(heading);
    });
    const shownHeading = () =>
      guest.evaluate(() => getComputedStyle(document.querySelector('[data-tonk-heading]')).display);
    assert.equal(await shownHeading(), 'none');
    await post({ type: 'tonk-size', size: 'full' });
    await guest.waitForFunction(() => document.documentElement.dataset.tonkSize === 'full');
    assert.equal(await shownHeading(), 'block');
  } finally {
    await browser.close();
  }
});
