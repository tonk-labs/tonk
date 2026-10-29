// The person: a fresh Chromium profile with a virtual passkey that signs
// up, confirms its email and approves the agent on `/settings/agent`.
//
// The account ceremony follows the Rust browser suite
// (`rust/tonk-ui/src/account_flow.rs`, `sign_up`): the same element ids,
// driven through Playwright instead of WebDriver. The app renders in sealed
// `srcdoc` frames; Playwright reaches into those over CDP, so locators
// chain through `tonk-site > iframe` the way `enter_guest` does.

import { chromium } from 'playwright';

const SETTLE = 150_000;

export async function launch({ headed = false, trace } = {}) {
  const browser = await chromium.launch({ headless: !headed });
  const context = await browser.newContext({ ignoreHTTPSErrors: true });
  if (trace) await context.tracing.start({ screenshots: true, snapshots: true });
  const page = await context.newPage();
  // A virtual authenticator with PRF, as `driver_with_prf` sets up: the
  // profile's key is derived from the passkey's PRF output.
  const cdp = await context.newCDPSession(page);
  await cdp.send('WebAuthn.enable');
  await cdp.send('WebAuthn.addVirtualAuthenticator', {
    options: {
      protocol: 'ctap2',
      ctap2Version: 'ctap2_1',
      transport: 'internal',
      hasResidentKey: true,
      hasUserVerification: true,
      isUserVerified: true,
      hasPrf: true,
      automaticPresenceSimulation: true,
    },
  });
  return {
    page,
    async close({ keepTrace } = {}) {
      if (trace) await context.tracing.stop(keepTrace ? { path: trace } : undefined);
      await browser.close();
    },
  };
}

/** The guest frame the Hub and settings pages render in. */
const guest = (page) => page.frameLocator('tonk-site > iframe');

/** Wait until the app has booted under its service worker. */
export async function boot(page, origin) {
  await page.goto(origin);
  await page.waitForFunction(
    () => !!window.tonkIdentity && !!navigator.serviceWorker?.controller,
    null,
    { timeout: SETTLE },
  );
}

/** Create an account for `address` and confirm it, leaving it able to host spaces. */
export async function signUp(page, { origin, address, inbox }) {
  await boot(page, origin);
  await guest(page).locator('.hub-page').waitFor({ timeout: SETTLE });
  const trigger = guest(page).locator('[data-account-trigger]');
  await trigger.filter({ hasText: 'add an account' }).waitFor({ timeout: 60_000 });
  await trigger.click();

  const field = page.locator('#tonk-register-email');
  await field.waitFor({ timeout: 30_000 });
  await field.fill(address);
  const action = page.locator('#tonk-register-action:not([hidden])');
  await action.filter({ hasText: 'create a passkey' }).waitFor({ timeout: 30_000 });
  const before = await page.evaluate(() => performance.timeOrigin);
  await action.click();

  const name = page
    .locator('#tonk-register .orow')
    .filter({
      has: page.locator('.k', { hasText: /^(display name|what should people call you\?)$/ }),
    })
    .locator('input');
  await name.waitFor({ timeout: 30_000 });
  await name.fill('E2E agent owner');
  await name.press('Enter');

  // Either the ceremony asks for the emailed link, or it promoted a fresh
  // profile and reloaded; both mean enrollment landed.
  await Promise.race([
    page
      .locator('#tonk-register-status')
      .filter({ hasText: 'confirmation link' })
      .waitFor({ timeout: 45_000 }),
    page.waitForFunction((t) => performance.timeOrigin !== t, before, { timeout: 45_000 }),
  ]);

  // Opened on the app's origin: a local access service names its own port
  // in the link, while a deployed one is the app's origin already.
  const link = new URL(await inbox.link(address));
  await page.goto(new URL(link.pathname + link.search, origin).toString());
  await page.locator('#activate-accept').click();
  await page.locator('#activate-done').waitFor({ state: 'visible', timeout: 60_000 });

  // Activation releases the account work queued behind it; wait for the
  // backup to settle before anything navigates away from it.
  await boot(page, origin);
  await page.waitForFunction(
    async () => {
      const read = async (path) => (await fetch(path)).json();
      await read('/api/customer');
      const pending = await read('/api/customer/pending');
      const account = await read('/api/account');
      return (
        !(Array.isArray(pending) && pending.some((work) => work.kind === 'publishCustody')) &&
        account.status === 'registered'
      );
    },
    null,
    { timeout: 90_000, polling: 500 },
  );
}

/**
 * Create a space through the app's own worker, as the Hub's "new space"
 * does (`space/create` on the active profile branch), and wait for the
 * profile to list it. The Hub's dialog is not what this test is about.
 */
export async function createSpace(page, { origin, name }) {
  await boot(page, origin);
  await page.evaluate(async (name) => {
    const { active } = await (await fetch('/api/profiles')).json();
    const response = await fetch(`/api/repository/profile:tonk/branch/${active}/transact`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        claims: [
          {
            op: 'assert',
            application: {
              predicate: {
                kind: 'transient',
                concept: {
                  description: 'A request to create a new space.',
                  with: { name: { the: 'xyz.tonk.command.create-space/name', as: 'Text' } },
                },
              },
              parameters: { name },
            },
          },
        ],
      }),
    });
    if (!response.ok) throw new Error(`space/create answered ${response.status}`);
  }, name);
}

/**
 * Wait until the profile lists a space labelled with each of `names` and
 * each has a remote (the approval page offers only synced spaces), and
 * return their subjects by name.
 */
export async function syncedSpaces(page, names, timeout = 120_000) {
  const read = () =>
    page.evaluate(async () => {
      const json = async (path) => (await fetch(path)).json();
      const spaces = [];
      for (const { key, subject } of (await json('/api/profile'))?.space ?? []) {
        const info = await json(`/api/repository/${encodeURIComponent(key)}`).catch(() => null);
        spaces.push({
          subject,
          label: info?.label,
          synced: Object.keys(info?.remote ?? {}).length > 0,
        });
      }
      return spaces;
    });
  const deadline = Date.now() + timeout;
  for (;;) {
    const spaces = await read().catch(() => []);
    const found = names.map((name) => spaces.find((space) => space.label === name && space.synced));
    if (found.every(Boolean)) {
      return Object.fromEntries(names.map((name, index) => [name, found[index].subject]));
    }
    if (Date.now() > deadline) {
      throw new Error(`spaces never synced: wanted ${names}, have ${JSON.stringify(spaces)}`);
    }
    await page.waitForTimeout(1000);
  }
}

/**
 * Approve the agent on the page `/authorize` sent the browser to, granting
 * `grants` (space name to "Read" or "Read and write"). The page then sends
 * the browser on to the client's callback.
 *
 * Locators are by role and accessible name, agreed with the approval page.
 */
export async function approve(page, { authorizationUrl, agent, grants }) {
  await page.goto(authorizationUrl.toString());
  const root = await approvalRoot(page);
  const name = root.getByRole('textbox', { name: /agent name/i });
  if (await name.count()) await name.fill(agent);
  for (const [space, access] of Object.entries(grants)) {
    await root
      .getByRole('combobox', { name: space })
      .selectOption({ label: access }, { timeout: 120_000 });
  }
  await root.getByRole('button', { name: 'Approve' }).click();
}

/** The approval page renders either in the top document or in the guest frame. */
async function approvalRoot(page) {
  const approve = (root) => root.getByRole('button', { name: 'Approve' });
  const deadline = Date.now() + SETTLE;
  for (;;) {
    if (await approve(page).count()) return page;
    if (await approve(guest(page)).count()) return guest(page);
    if (Date.now() > deadline) throw new Error('the approval page never offered Approve');
    await page.waitForTimeout(500);
  }
}
