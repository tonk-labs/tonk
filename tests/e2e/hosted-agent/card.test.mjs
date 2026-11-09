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
        { mark: '+', name: 'Towels', kind: 'view' },
        { mark: '~', name: 'Sunscreen' },
      ],
      lines: '+ Pack towels.',
      ledger: [
        { term: 'in', value: 'Family trip' },
        { term: 'changed by', value: 'you', bot: true },
      ],
      fine: 'From the Tonk library.',
      answers: [
        { label: 'change 2 items', kind: 'main', tool: 'tonk_proposal_apply', arguments: HELD },
        { label: 'keep as is', kind: 'ring', tool: 'tonk_proposal_dismiss', arguments: HELD },
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
  tonk_proposal_apply: strip('changed 2 items in Family trip', true),
  tonk_proposal_dismiss: strip('nothing changed · Family trip stays as it is', false),
};

const tell = {
  isError: true,
  content: [{ type: 'text', text: 'This connection has no access.' }],
  structuredContent: {
    card: {
      kind: 'tell',
      space: { did: SPACE, name: 'Family trip' },
      disc: 'filled',
      label: 'no access to Family trip',
      say: 'I can’t open Family trip: this agent has no access to it. You can grant it from your agents in Tonk.',
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
        look: b.className,
        x: b.getBoundingClientRect().x,
      })),
      receiptInHead: !!head?.querySelector('.tonk-receipt'),
      ledgerColumns: (() => {
        const ledger = card.querySelector('.tonk-ledger');
        return ledger ? getComputedStyle(ledger).gridTemplateColumns.split(' ').length : 0;
      })(),
      height: card.getBoundingClientRect().height,
      background: getComputedStyle(card).backgroundColor,
    };
  });
}

// The strip's receipt, once it says something other than `past` (the
// "saving" strip an answer shows first).
async function receipt(frame, past = 'saving') {
  for (let tries = 0; tries < 50; tries++) {
    const shown = await frame.evaluate((past) => {
      const receipt = document.querySelector('.tonk-strip .tonk-receipt');
      if (!receipt?.textContent || receipt.textContent === past) return null;
      return {
        text: receipt.textContent,
        status: receipt.getAttribute('role'),
        bot: !!receipt.querySelector('.tonk-bot'),
        focused: document.activeElement === receipt,
      };
    }, past);
    if (shown) return shown;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error('no strip');
}

test('an ask draws its request, what changes and its answers, main first', async () => {
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
    assert.deepEqual(card.changes, ['+Towelsview', '~Sunscreen']);
    assert.equal(card.lines, '+ Pack towels.');
    assert.deepEqual(card.ledger, ['Family trip', 'youbot']);
    assert.equal(card.fine, 'From the Tonk library.');
    assert.deepEqual(
      card.buttons.map(({ text, look }) => ({ text, look })),
      [
        { text: 'change 2 items', look: 'tonk-btn' },
        { text: 'keep as is', look: 'tonk-btn tonk-btn--ring' },
      ],
    );
    assert.ok(card.buttons[0].x < card.buttons[1].x, 'the main answer is not on the left');
    assert.equal(card.receiptInHead, false, 'an empty receipt in the head');
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
    assert.equal(shownReceipt.text, 'changed 2 items in Family trip · by youbot');
    assert.equal(shownReceipt.status, 'status');
    assert.equal(shownReceipt.bot, true);
    assert.equal(shownReceipt.focused, true, 'focus did not move to the receipt');
    assert.deepEqual(shown.calls, [{ name: 'tonk_proposal_apply', arguments: HELD }]);
    const context = (await shown.asked()).filter((m) => m.method === 'ui/update-model-context');
    assert.equal(context.length, 1);
    assert.equal(
      context[0].params.content[0].text,
      'The person answered: changed 2 items in Family trip.',
    );
  } finally {
    await shown.close();
  }
});

test('keep as is dismisses, and its receipt names no agent', async () => {
  const shown = await showCard({ result: ask, answers: ANSWERS });
  try {
    await shown.card().click('button:text-is("keep as is")');
    const shownReceipt = await receipt(shown.card());
    assert.equal(shownReceipt.text, 'nothing changed · Family trip stays as it is');
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
    assert.equal(card.label, 'Tonk: no access to Family trip');
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

test('an answer shows a saving strip until the space answers', async () => {
  const shown = await showCard({ result: ask, answers: ANSWERS, delay: 1500 });
  try {
    await shown.card().click('button:text-is("change 2 items")');
    const saving = await receipt(shown.card(), null);
    assert.equal(saving.text, 'saving');
    assert.equal(
      await shown.card().evaluate(() => !!document.querySelector('.tonk--ask')),
      false,
      'the ask stays while saving',
    );
    await shown.card().waitForFunction(() =>
      document.querySelector('.tonk-strip .tonk-receipt')?.textContent.startsWith('changed'),
    );
  } finally {
    await shown.close();
  }
});

test('a change not saved offers try again and drop it, and drop it calls nothing', async () => {
  const unsaved = {
    isError: true,
    content: [{ type: 'text', text: 'PULL_FAILED' }],
    structuredContent: {
      card: {
        kind: 'tell',
        space: { did: SPACE, name: 'Family trip' },
        role: 'write',
        disc: 'hollow',
        label: 'not saved',
        say: 'This change isn’t saved because Tonk can’t reach Family trip. Nothing in it changed.',
        answers: [
          { label: 'try again', kind: 'main', receipt: 'asked to try again', context: 'Try again.' },
          { label: 'drop it', kind: 'link', receipt: 'dropped · nothing changed in Family trip', context: 'Dropped.' },
        ],
        agent: AGENT,
        person: PERSON,
      },
    },
  };
  const shown = await showCard({ result: unsaved });
  try {
    const card = await drawn(shown.card());
    assert.deepEqual(
      card.buttons.map(({ text, look }) => ({ text, look })),
      [
        { text: 'try again', look: 'tonk-btn' },
        { text: 'drop it', look: 'tonk-link' },
      ],
    );
    await shown.card().click('button:text-is("drop it")');
    assert.equal((await receipt(shown.card())).text, 'dropped · nothing changed in Family trip');
    assert.deepEqual(shown.calls, []);
    const context = (await shown.asked()).filter((m) => m.method === 'ui/update-model-context');
    assert.equal(context[0].params.content[0].text, 'Dropped.');
  } finally {
    await shown.close();
  }
});

test('the ledger keeps two columns on a narrow card', async () => {
  const shown = await showCard({ result: ask, width: 360 });
  try {
    const card = await drawn(shown.card());
    assert.equal(card.ledgerColumns, 2);
  } finally {
    await shown.close();
  }
});

test('a tell can open Tonk through the agent app, and stays as it is', async () => {
  const readOnly = {
    isError: true,
    content: [{ type: 'text', text: 'read only' }],
    structuredContent: {
      card: {
        kind: 'tell',
        space: { did: SPACE, name: 'Family trip' },
        role: 'read',
        disc: 'filled',
        label: 'read only in Family trip',
        say: 'I can only read Family trip, so I didn’t change it.',
        ledger: [{ term: 'this agent', value: 'read · sees and searches only' }],
        answers: [
          { label: 'open in tonk ↗', kind: 'main', url: `https://tonk.example/space/${SPACE}` },
          { label: 'skip', kind: 'ring', receipt: 'nothing changed · Family trip stays as it is', context: 'Skipped.' },
        ],
        agent: AGENT,
        person: PERSON,
      },
    },
  };
  const shown = await showCard({ result: readOnly });
  try {
    const card = await drawn(shown.card());
    assert.equal(card.label, 'Tonk: read only in Family trip');
    assert.equal(
      await shown.card().evaluate(() => document.querySelector('.tonk--tell').getAttribute('role')),
      null,
      'a tell is not an alert',
    );
    await shown.card().click('button:text-is("open in tonk ↗")');
    await shown.card().waitForTimeout(300);
    const opened = (await shown.asked()).filter((m) => m.method === 'ui/open-link');
    assert.deepEqual(opened.map((m) => m.params.url), [`https://tonk.example/space/${SPACE}`]);
    assert.equal((await drawn(shown.card())).classes.includes('tonk--tell'), true);
    assert.deepEqual(shown.calls, []);
  } finally {
    await shown.close();
  }
});
