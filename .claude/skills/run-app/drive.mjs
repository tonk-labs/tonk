// Drive the served app in headless Chromium. Usage:
//   node drive.mjs [url]        — load, wait for onboarding, screenshot, list frames
// Import `open` / `frameWith` from here in task-specific scripts.
import { chromium } from 'playwright';

export async function open(url = 'http://localhost:8080/', settle = 30000) {
  const browser = await chromium.launch({ args: ['--no-sandbox'] });
  const page = await (await browser.newContext({ viewport: { width: 1280, height: 800 } })).newPage();
  const logs = [];
  page.on('console', (m) => { if (m.type() === 'error') logs.push(m.text().slice(0, 300)); });
  page.on('pageerror', (e) => logs.push(String(e).slice(0, 300)));
  await page.goto(url);
  await page.waitForTimeout(settle);
  return { browser, page, logs };
}

// The app is nested sealed iframes (page → profile chrome guest → space
// guest). Guests remount, so re-find the frame before each step.
export async function frameWith(page, predicate, tries = 30) {
  for (let i = 0; i < tries; i++) {
    for (const frame of page.frames()) {
      if (await frame.evaluate(predicate).catch(() => false)) return frame;
    }
    await page.waitForTimeout(1000);
  }
  throw new Error('no frame matched');
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const { browser, page, logs } = await open(process.argv[2]);
  await page.screenshot({ path: process.env.OUT || 'screenshot.png' });
  for (const frame of page.frames()) console.log('frame', frame.url().slice(0, 80));
  console.log(logs.join('\n'));
  await browser.close();
}
