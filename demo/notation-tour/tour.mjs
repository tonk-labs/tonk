// The notation tour, driven end to end in headless Chromium.
//
// Needs the app served on :8080 (`.claude/skills/run-app/dev-web.sh`).
// Run from this directory:
//
//   ln -sf "$(npm root -g)" node_modules   # Playwright is installed globally
//   node tour.mjs
//
// It follows the hub's link to the tour's seed page, creates the "Notation tour" space,
// waits for the tour notebook, runs every ```notation cell in order and
// checks each one's outcome against what the tour says it should be (ran,
// or refused for a cell marked `; expect: refused`), edits one cell and
// runs it again, and saves a screenshot of each section to ./shots/.
//
// Environment: BASE (default http://localhost:8080), SHOTS (default ./shots).
import { chromium } from 'playwright';
import fs from 'node:fs';
import path from 'node:path';

const BASE = process.env.BASE || 'http://localhost:8080';
const SHOTS = process.env.SHOTS || path.join(path.dirname(new URL(import.meta.url).pathname), 'shots');
fs.mkdirSync(SHOTS, { recursive: true });

const log = (...args) => console.log(new Date().toISOString().slice(11, 19), ...args);

async function frameWith(page, predicate, arg, seconds = 60) {
  for (let i = 0; i < seconds; i++) {
    for (const frame of page.frames()) {
      if (await frame.evaluate(predicate, arg).catch(() => false)) return frame;
    }
    await page.waitForTimeout(1000);
  }
  throw new Error(`no frame matched ${predicate}`);
}

// The prose editor renders in a shadow root; open shadow roots are pierced by
// Playwright's CSS engine, but evaluate() needs a deep query.
const deepAll = (selector) => {
  const out = [];
  const walk = (root) => {
    root.querySelectorAll(selector).forEach((el) => out.push(el));
    root.querySelectorAll('*').forEach((el) => el.shadowRoot && walk(el.shadowRoot));
  };
  walk(document);
  return out;
};

const browser = await chromium.launch({ args: ['--no-sandbox'] });
const context = await browser.newContext({ viewport: { width: 1280, height: 1000 } });
const page = await context.newPage();
const errors = [];
page.on('pageerror', (e) => errors.push(String(e).slice(0, 300)));
let failed = 0;
const check = (ok, what) => {
  if (ok) log('ok  ', what);
  else {
    failed++;
    log('FAIL', what);
  }
};

try {
  // A fresh profile lands in onboarding first; let it settle.
  log('opening the app');
  await page.goto(BASE);
  await page.waitForTimeout(30000);

  // Reach the tour by clicking, as a person on the preview would: the hub
  // links to it from its troubleshooting fold, beside `make a space`.
  log('opening the hub and following its link to the notation tour');
  await page.goto(`${BASE}/`);
  const hub = await frameWith(page, () => !!document.querySelector('a[href*="notation-tour"]'));
  const via = await hub.evaluate(async () => {
    // Open the fold as a person would: troubleshooting, then the question.
    // The hub's script may still be starting, so try until each opens.
    const wait = () => new Promise((r) => setTimeout(r, 500));
    const open = async (id) => {
      for (let i = 0; i < 40 && document.querySelector(`#${id}`)?.inert; i++) {
        if (document.querySelector(`[aria-controls="${id}"]`)?.getAttribute('aria-expanded') !== 'true') {
          document.querySelector(`[aria-controls="${id}"]`)?.click();
        }
        await wait();
      }
    };
    await open('help-body');
    await open('help-without');
    const link = document.querySelector('#help-without a[href*="notation-tour"]');
    const opened = !document.querySelector('#help-without')?.closest('[inert]');
    if (!link || !opened) return 'nothing';
    link.click();
    return 'the troubleshooting fold';
  });
  log('followed the link in', via);
  check(via !== 'nothing', 'the hub links to the notation tour');
  const dialog = await frameWith(page, () => !!document.querySelector('space-create[autoopen] input[name=name]'));
  await dialog.fill('space-create[autoopen] input[name=name]', 'Notation tour');
  await page.screenshot({ path: path.join(SHOTS, '00-create.png') });
  await dialog.click('space-create[autoopen] [data-space-create-submit]');

  log('waiting for the tour notebook');
  const deep = `(${deepAll.toString()})`;
  const notebook = await frameWith(
    page,
    (deep) => eval(deep)('.md-code-block[data-cell-language=notation]').length > 5,
    deep,
    180,
  );
  const cells = await notebook.evaluate((deep) => eval(deep)('.md-code-block[data-cell-language=notation]').length, deep);
  log(`the notebook shows ${cells} notation cells`);
  check(cells > 10, 'the tour notebook renders its notation cells');
  await page.screenshot({ path: path.join(SHOTS, '01-notebook.png') });

  // Run each cell in order, and check its outcome.
  const results = [];
  let live = null;
  for (let at = 0; at < cells; at++) {
    const outcome = await notebook.evaluate(
      async ({ deep, at }) => {
        const all = eval(deep)('.md-code-block[data-cell-language=notation]');
        const cell = all[at];
        const source = cell.querySelector('tonk-code').value || '';
        cell.scrollIntoView({ block: 'center' });
        cell.querySelector('.notation-cell-run').click();
        for (let i = 0; i < 300; i++) {
          await new Promise((r) => setTimeout(r, 100));
          const state = cell.getAttribute('data-cell-state');
          if (state && state !== 'running') {
            const result = cell.querySelector('.notation-cell-result');
            return {
              source,
              state,
              forms: [...result.querySelectorAll('.notation-form')].map((f) => ({
                kind: f.dataset.kind,
                outcome: f.dataset.outcome,
                head: f.querySelector('.notation-form-head')?.textContent.trim(),
                rows: f.querySelectorAll('.notation-rows tbody tr').length,
                printed: f.querySelector('.notation-printed')?.textContent.trim().split('\n').filter(Boolean).length || 0,
                html: !!f.querySelector('.notation-html-rendered'),
                warnings: f.querySelectorAll('.notation-warning').length,
              })),
              stages: [...result.querySelectorAll('.notation-stage')].map((s) => s.dataset.stage),
            };
          }
        }
        return { source, state: 'timeout', forms: [], stages: [] };
      },
      { deep, at },
    );
    results.push(outcome);
    if (outcome.forms.some((f) => f.html) && !live) {
      // Watch the view live: the first frame is the driver's render.
      live = await notebook.evaluate(
        async ({ deep, at }) => {
          const cell = eval(deep)('.md-code-block[data-cell-language=notation]')[at];
          cell.querySelector('.notation-watch').click();
          for (let i = 0; i < 100; i++) {
            await new Promise((r) => setTimeout(r, 100));
            const view = cell.querySelector('.notation-live');
            if (view && Number(view.dataset.frames) > 0) {
              return { at, items: view.querySelectorAll('li').length, frames: Number(view.dataset.frames) };
            }
          }
          return { at, items: 0, frames: 0 };
        },
        { deep, at },
      );
      check(live.frames === 1 && live.items > 0, `the view is watched live: ${live.items} item(s) in its first frame`);
    }
    const expected = outcome.source.includes('; expect: refused') ? 'refused' : 'ran';
    // A refused cell answers nothing, a refused select included.
    const answered = expected === 'refused' || outcome.forms
      .filter((f) => f.kind === 'select' || f.kind === 'query')
      .every((f) => f.rows + f.printed + (f.html ? 1 : 0) > 0 || f.kind === 'query');
    const warned = !outcome.source.includes('; expect: warning') || outcome.forms.some((f) => f.warnings > 0);
    check(
      outcome.state === expected && answered && warned && outcome.stages.length === 4,
      `cell ${at + 1}: ${outcome.state} (${outcome.forms.map((f) => `${f.kind}: ${f.head}`).join('; ')})`,
    );
  }

  // The watched view followed the cells run after it, by patches.
  if (live) {
    await page.waitForTimeout(1000);
    const after = await notebook.evaluate(
      ({ deep, at }) => {
        const view = eval(deep)('.md-code-block[data-cell-language=notation]')[at].querySelector('.notation-live');
        view.scrollIntoView({ block: 'center' });
        return {
          items: [...view.querySelectorAll('li')].map((li) => li.textContent),
          frames: Number(view.dataset.frames),
          patches: Number(view.dataset.patches),
          missed: view.dataset.missed || null,
          error: view.dataset.error || null,
          lastMs: view.dataset.lastMs || null,
        };
      },
      { deep, at: live.at },
    );
    log('live view after the later cells:', JSON.stringify(after));
    check(
      after.items.length > live.items && after.items.includes('Bake bread') && after.patches > 0 && !after.missed && !after.error,
      "a later cell's write reached the watched view as patches",
    );
    await page.screenshot({ path: path.join(SHOTS, '92-live.png') });

    // An event from the page: click the open to-do in the watched card.
    // Its bound command posts as a notation cell, and the write comes
    // back to the card as a patch of the item's class.
    const clicked = await notebook.evaluate(
      async ({ deep, at }) => {
        const view = eval(deep)('.md-code-block[data-cell-language=notation]')[at].querySelector('.notation-live');
        const item = [...view.querySelectorAll('li')].find((li) => li.textContent === 'Bake bread');
        const before = { cls: item.getAttribute('class'), patches: Number(view.dataset.patches) };
        const clickedAt = performance.now();
        item.click();
        for (let i = 0; i < 500; i++) {
          await new Promise((r) => setTimeout(r, 20));
          const now = [...view.querySelectorAll('li')].find((li) => li.textContent === 'Bake bread');
          if (now && now.getAttribute('class') !== before.cls) {
            return { before, after: now.getAttribute('class'), patches: Number(view.dataset.patches), posted: view.dataset.posted, same: now === item, ms: Math.round(performance.now() - clickedAt) };
          }
        }
        return { before, after: null, posted: view.dataset.posted || null, error: view.dataset.error || null };
      },
      { deep, at: live.at },
    );
    log('click in the live view:', JSON.stringify(clicked));
    check(
      clicked.before.cls === 'open' && clicked.after === 'done' && clicked.patches > clicked.before.patches && clicked.same,
      'a click in the watched view posted its command and came back as a patch',
    );
    await page.screenshot({ path: path.join(SHOTS, '93-clicked.png') });
  }

  // The views cell rendered HTML; the refusals said where.
  check(results.some((r) => r.forms.some((f) => f.html)), 'a view rendered its HTML in a cell');
  check(
    results.filter((r) => r.state === 'refused').length >= 3,
    'the refusals section refuses each of its cells',
  );

  // A screenshot per section: scroll each heading into view.
  const headings = await notebook.evaluate((deep) => eval(deep)('h2').map((h) => h.textContent.trim()), deep);
  for (const [i, heading] of headings.entries()) {
    await notebook.evaluate(
      ({ deep, i }) => eval(deep)('h2')[i].scrollIntoView({ block: 'start' }),
      { deep, i },
    );
    await page.waitForTimeout(300);
    const name = `${String(i + 2).padStart(2, '0')}-${heading.toLowerCase().replace(/[^a-z]+/g, '-').replace(/-$/, '')}.png`;
    await page.screenshot({ path: path.join(SHOTS, name) });
    log('screenshot', name);
  }

  // The pipeline: open the stages of the first query cell.
  await notebook.evaluate((deep) => {
    const cell = eval(deep)('.md-code-block[data-cell-language=notation]')[2];
    cell.querySelectorAll('details.notation-stage').forEach((d) => (d.open = true));
    cell.scrollIntoView({ block: 'start' });
  }, deep);
  await page.waitForTimeout(300);
  await page.screenshot({ path: path.join(SHOTS, '90-pipeline.png') });

  // Edit a cell and run it again: the first query asks for open to-dos only.
  log('editing the first query cell and running it again');
  const edited = await notebook.evaluate(async (deep) => {
    const cell = eval(deep)('.md-code-block[data-cell-language=notation]')[2];
    const editor = cell.querySelector('tonk-code');
    editor.value = '(select (todo ?t title: ?title state: :open list: ?list))';
    cell.querySelector('.notation-cell-run').click();
    await new Promise((r) => setTimeout(r, 300));
    for (let i = 0; i < 300; i++) {
      await new Promise((r) => setTimeout(r, 100));
      const state = cell.getAttribute('data-cell-state');
      if (state && state !== 'running') {
        return {
          state,
          rows: [...cell.querySelectorAll('.notation-rows tbody tr')].map((tr) => tr.textContent),
        };
      }
    }
    return { state: 'timeout', rows: [] };
  }, deep);
  log('edited cell:', JSON.stringify(edited));
  check(
    edited.state === 'ran' && edited.rows.length > 0 && edited.rows.every((row) => !row.includes('done')),
    'an edited cell runs again and shows the new answer',
  );
  await page.screenshot({ path: path.join(SHOTS, '91-edited.png') });

  fs.writeFileSync(path.join(SHOTS, 'results.json'), JSON.stringify(results, null, 2));
} catch (error) {
  failed++;
  log('FAIL', error.message);
  await page.screenshot({ path: path.join(SHOTS, 'failure.png') }).catch(() => {});
} finally {
  if (errors.length) log('page errors:', errors.slice(0, 10).join('\n'));
  await browser.close();
}
log(failed ? `${failed} check(s) failed` : 'every check passed');
process.exit(failed ? 1 : 0);
