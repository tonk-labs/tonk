// Tonk's card alone, in a real browser, as an MCP Apps host shows it: an
// ask, its answers, the strip that replaces it, a tell, and the host's
// theme. The host and the provider are stand-ins (lib/card-host.mjs), so
// no deployment, account or space is needed:
//
//   npm run test:card
//
// What it cannot show: a real host's layout and policies, and the tools
// behind the answers (the Rust tests in router/hosted cover those).

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { showCard } from './lib/card-host.mjs';

const SPACE = 'did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK';
const AGENT = { did: 'did:key:z6MkAgent', name: 'Juno' };
const PERSON = { name: null };
const HELD = { space: SPACE, id: 'held-1' };

// What `tonk_evaluate` with `confirm: true` answers.
const ask = {
  content: [{ type: 'text', text: 'Asked the person to confirm in a card.' }],
  structuredContent: {
    card: {
      kind: 'ask',
      space: { did: SPACE, name: 'Family trip', item: 'Packing list' },
      role: 'write',
      disc: 'filled',
      say: 'change 2 items in Family trip',
      changes: [
        { mark: '+', name: 'Towels', kind: 'item' },
        { mark: '~', name: 'Sunscreen', kind: 'item' },
      ],
      lines: '+ Pack towels.',
      ledger: [
        { term: 'in', value: 'Family trip' },
        { term: 'made by', value: 'Juno', bot: true },
      ],
      fine: 'Nothing changes until you answer.',
      answers: [
        { label: 'not now', tool: 'tonk_proposal_dismiss', arguments: HELD },
        { label: 'change 2 items', main: true, tool: 'tonk_proposal_apply', arguments: HELD },
      ],
      agent: AGENT,
      person: PERSON,
    },
  },
};

const strip = (say, byAgent) => ({
  content: [{ type: 'text', text: `The person answered: ${say}.` }],
  structuredContent: {
    card: {
      kind: 'strip',
      space: { did: SPACE, name: 'Family trip' },
      role: 'write',
      disc: 'filled',
      say,
      byAgent,
      agent: AGENT,
      person: PERSON,
    },
  },
});

const ANSWERS = {
  tonk_proposal_apply: strip('saved 2 items in Family trip', true),
  tonk_proposal_dismiss: strip('kept as is · nothing changed in Family trip', false),
};

const tell = {
  isError: true,
  content: [{ type: 'text', text: 'This connection has no access.' }],
  structuredContent: {
    card: {
      kind: 'tell',
      space: { did: SPACE, name: 'Family trip' },
      disc: 'filled',
      label: 'no access',
      say: 'This connection has no access to Family trip.',
      agent: AGENT,
      person: PERSON,
    },
  },
};

async function drawn(frame) {
  return frame.evaluate(() => {
    const card = document.querySelector('.tonk');
    const head = card.querySelector('.tonk-head');
    return {
      classes: card.className,
      label: card.getAttribute('aria-label'),
      disc: !!head?.querySelector('.tonk-disc'),
      role: head?.querySelector('.tonk-role')?.textContent ?? null,
      space: head?.querySelector('.tonk-space')?.textContent ?? null,
      where: head?.querySelector('.tonk-where')?.textContent ?? null,
      say: card.querySelector('.tonk-say')?.textContent ?? null,
      changes: [...card.querySelectorAll('.tonk-changes li')].map((li) => li.textContent),
      lines: card.querySelector('.tonk-lines')?.textContent ?? null,
      ledger: [...card.querySelectorAll('.tonk-ledger dd')].map((dd) => dd.textContent),
      fine: card.querySelector('.tonk-fine')?.textContent ?? null,
      buttons: [...card.querySelectorAll('.tonk-acts button')].map((b) => ({
        text: b.textContent,
        ring: b.classList.contains('tonk-btn--ring'),
        x: b.getBoundingClientRect().x,
      })),
      background: getComputedStyle(card).backgroundColor,
    };
  });
}

async function receipt(frame) {
  for (let tries = 0; tries < 50; tries++) {
    const shown = await frame.evaluate(() => {
      const receipt = document.querySelector('.tonk-strip .tonk-receipt');
      if (!receipt?.textContent) return null;
      return {
        text: receipt.textContent,
        status: receipt.getAttribute('role'),
        bot: !!receipt.querySelector('.tonk-bot'),
        focused: document.activeElement === receipt,
      };
    });
    if (shown) return shown;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error('no strip');
}

test('an ask draws its request, what changes and its answers, safe first', async () => {
  const shown = await showCard({ result: ask, answers: ANSWERS });
  try {
    const card = await drawn(shown.card());
    assert.match(card.classes, /tonk--ask/);
    assert.equal(card.label, 'Tonk asks: change 2 items in Family trip');
    assert.equal(card.disc, true);
    assert.equal(card.role, 'write');
    assert.equal(card.space, 'Family trip');
    assert.equal(card.where, '› Packing list');
    assert.equal(card.say, 'change 2 items in Family trip');
    assert.deepEqual(card.changes, ['+Towelsitem', '~Sunscreenitem']);
    assert.equal(card.lines, '+ Pack towels.');
    assert.deepEqual(card.ledger, ['Family trip', 'Junobot']);
    assert.equal(card.fine, 'Nothing changes until you answer.');
    assert.deepEqual(
      card.buttons.map(({ text, ring }) => ({ text, ring })),
      [
        { text: 'not now', ring: true },
        { text: 'change 2 items', ring: false },
      ],
    );
    assert.ok(card.buttons[0].x < card.buttons[1].x, 'the safe answer is not on the left');
    assert.equal(card.background, 'rgb(56, 24, 42)');
    assert.deepEqual(shown.violations, []);
  } finally {
    await shown.close();
  }
});

test('applying swaps the ask for a receipt by you and the bot, and tells the model', async () => {
  const shown = await showCard({ result: ask, answers: ANSWERS });
  try {
    await shown.card().click('button:text-is("change 2 items")');
    const shownReceipt = await receipt(shown.card());
    assert.equal(shownReceipt.text, 'saved 2 items in Family trip · by youbot');
    assert.equal(shownReceipt.status, 'status');
    assert.equal(shownReceipt.bot, true);
    assert.equal(shownReceipt.focused, true, 'focus did not move to the receipt');
    assert.deepEqual(shown.calls, [{ name: 'tonk_proposal_apply', arguments: HELD }]);
    const context = (await shown.asked()).filter((m) => m.method === 'ui/update-model-context');
    assert.equal(context.length, 1);
    assert.equal(
      context[0].params.content[0].text,
      'The person answered: saved 2 items in Family trip.',
    );
  } finally {
    await shown.close();
  }
});

test('not now dismisses, and its receipt names no agent', async () => {
  const shown = await showCard({ result: ask, answers: ANSWERS });
  try {
    await shown.card().click('button:text-is("not now")');
    const shownReceipt = await receipt(shown.card());
    assert.equal(shownReceipt.text, 'kept as is · nothing changed in Family trip');
    assert.equal(shownReceipt.bot, false);
    assert.deepEqual(shown.calls, [{ name: 'tonk_proposal_dismiss', arguments: HELD }]);
  } finally {
    await shown.close();
  }
});

test('an answer the space refuses becomes the tell it sends', async () => {
  const gone = {
    isError: true,
    content: [{ type: 'text', text: 'Already answered.' }],
    structuredContent: {
      card: {
        kind: 'tell',
        space: { did: SPACE, name: 'Family trip' },
        role: 'write',
        disc: 'filled',
        label: 'not applied',
        say: 'This was already answered, or waited too long. Nothing changed. Ask me again.',
        agent: AGENT,
        person: PERSON,
      },
    },
  };
  const shown = await showCard({ result: ask, answers: { tonk_proposal_apply: gone } });
  try {
    await shown.card().click('button:text-is("change 2 items")');
    await shown.card().waitForSelector('.tonk--tell');
    const card = await drawn(shown.card());
    assert.equal(card.label, 'Tonk: not applied');
    assert.equal(card.role, 'write');
    assert.equal(
      await shown.card().evaluate(() => document.activeElement?.classList.contains('tonk--tell')),
      true,
    );
  } finally {
    await shown.close();
  }
});

test('a tell where the person is not in the space shows no disc and no role', async () => {
  const shown = await showCard({ result: tell });
  try {
    const card = await drawn(shown.card());
    assert.match(card.classes, /tonk--tell/);
    assert.equal(card.label, 'Tonk: no access');
    assert.equal(card.disc, false);
    assert.equal(card.role, null);
    assert.equal(card.space, 'Family trip');
    assert.deepEqual(card.buttons, []);
  } finally {
    await shown.close();
  }
});

test('the card follows the host theme, and an ask stays on ink', async () => {
  const shown = await showCard({ result: tell, context: { theme: 'dark' } });
  try {
    const dark = await drawn(shown.card());
    assert.equal(dark.background, 'rgb(35, 20, 28)');
    await shown.theme('light');
    await shown.card().waitForFunction(() => document.documentElement.dataset.theme === 'light');
    assert.equal((await drawn(shown.card())).background, 'rgb(247, 246, 245)');
  } finally {
    await shown.close();
  }
  const inked = await showCard({ result: ask, context: { theme: 'dark' } });
  try {
    assert.equal((await drawn(inked.card())).background, 'rgb(56, 24, 42)');
  } finally {
    await inked.close();
  }
});
