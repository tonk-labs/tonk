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
