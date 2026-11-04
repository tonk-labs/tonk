// A stand-in MCP Apps host for Tonk's card (`ui://tonk/card-v1.html`):
// asks, tells and strips, with no deployment behind them.
//
// Like `view-host.mjs`, the chat page frames a sandbox page on another
// origin, which renders the card's HTML in a `srcdoc` frame under a
// Content-Security-Policy built from the resource's `_meta.ui.csp`. It
// answers `ui/initialize` with `context` (the host's theme), sends the
// tool result, and answers the card's `tools/call`s from `answers` (tool
// name -> the result to give). It records what the card asked of the
// host, `ui/update-model-context` among it.
//
// It also serves the card's resource as `card::read_resource` does, from
// the files in the repo, and the fonts it names, with CORS, as the
// provider serves them. What it cannot stand in for: how Claude or
// ChatGPT lay a card out, and their policies beyond the declared domains.

import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';

const ROOT = fileURLToPath(new URL('../../../../', import.meta.url));
const HOSTED = `${ROOT}rust/tonk-worker/src/router/hosted/`;
const FONTS = `${ROOT}rust/tonk-ui/assets/in-agent/fonts/`;

const CHAT = (sandbox) => `<!doctype html><meta charset="utf-8"><title>chat</title>
<iframe id="card" src="${sandbox}" sandbox="allow-scripts allow-same-origin allow-forms"
  style="width:520px;height:600px;border:0"></iframe>`;

const SANDBOX = `<!doctype html><meta charset="utf-8"><title>sandbox</title>
<style>html,body,iframe{margin:0;width:100%;border:0}</style>
<iframe id="app" sandbox="allow-scripts allow-same-origin allow-forms"></iframe>
<script>
  const app = document.getElementById('app');
  let shown;
  window.asked = [];
  const send = (message) => app.contentWindow.postMessage({ jsonrpc: '2.0', ...message }, '*');
  addEventListener('message', async (event) => {
    const m = event.data;
    if (event.source !== app.contentWindow || m?.jsonrpc !== '2.0' || !m.method) return;
    const reply = (result) => send({ id: m.id, result });
    if (m.method === 'ui/initialize') {
      reply({
        protocolVersion: '2026-01-26',
        hostInfo: { name: 'tonk-e2e-host', version: '0' },
        hostCapabilities: { sandbox: { csp: shown.csp } },
        hostContext: shown.context,
      });
    } else if (m.method === 'ui/notifications/size-changed') {
      window.sized = m.params?.height;
      app.style.height = window.sized + 'px';
    } else if (m.method === 'ui/notifications/initialized') {
      send({ method: 'ui/notifications/tool-result', params: shown.result });
    } else if (m.method === 'tools/call') {
      window.asked.push({ method: m.method, params: m.params });
      reply(await window.hostCall(m.params.name, m.params.arguments ?? {}));
    } else if (m.id !== undefined) {
      window.asked.push({ method: m.method, params: m.params });
      reply({});
    }
  });
  window.show = (card) => {
    shown = card;
    app.srcdoc = card.html;
  };
  window.changeTheme = (theme) =>
    send({ method: 'ui/notifications/host-context-changed', params: { theme } });
</script>`;

// The policy a host builds from `_meta.ui.csp`, as `view-host.mjs` does.
function policy(csp) {
  const resources = (csp.resourceDomains ?? []).join(' ');
  return [
    "default-src 'none'",
    `script-src 'self' 'unsafe-inline' ${resources}`,
    `style-src 'self' 'unsafe-inline' ${resources}`,
    `img-src 'self' data: ${resources}`,
    `font-src 'self' data: ${resources}`,
    `connect-src 'self' ${(csp.connectDomains ?? []).join(' ')}`,
    "frame-src 'none'",
  ].join('; ');
}

// The card's resource as `card::read_resource` serves it for `origin`.
// (`in_agent::splice` also adds font preloads; they change nothing a
// test reads.)
async function cardResource(origin) {
  const css = (await readFile(`${HOSTED}in-agent/in-agent.css`, 'utf8')).replaceAll(
    '__TONK_IN_AGENT_FONTS__',
    `${origin}/in-agent/fonts`,
  );
  const html = (await readFile(`${HOSTED}card/card.html`, 'utf8')).replace(
    '<!-- tonk:in-agent -->',
    () => `<style>\n${css}</style>`,
  );
  return { html, csp: { connectDomains: [], resourceDomains: [origin] } };
}

/**
 * Show a tool `result` whose `structuredContent.card` the card draws, in
 * a fresh browser, with the host's `context`. `answers` gives the result
 * of each tool the card calls.
 */
export async function showCard({ result, answers = {}, context = { theme: 'light' }, headed }) {
  const fonts = createServer(async (request, response) => {
    const name = new URL(request.url, 'http://fonts').pathname.replace('/in-agent/fonts/', '');
    try {
      const body = await readFile(FONTS + name.replace(/[^a-z0-9.-]/g, ''));
      response.writeHead(200, {
        'content-type': 'font/woff2',
        'access-control-allow-origin': '*',
      });
      response.end(body);
    } catch {
      response.writeHead(404);
      response.end();
    }
  });
  await new Promise((resolve) => fonts.listen(0, '127.0.0.1', resolve));
  const origin = `http://127.0.0.1:${fonts.address().port}`;
  const resource = await cardResource(origin);
  const server = createServer((request, response) => {
    const { pathname } = new URL(request.url, 'http://host');
    if (pathname === '/sandbox') {
      response.writeHead(200, {
        'content-type': 'text/html',
        'content-security-policy': policy(resource.csp),
      });
      return response.end(SANDBOX);
    }
    response.writeHead(200, { 'content-type': 'text/html' });
    response.end(CHAT(`http://localhost:${server.address().port}/sandbox`));
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const browser = await chromium.launch({
    headless: !headed,
    channel: process.env.TONK_E2E_CHANNEL || undefined,
  });
  const page = await browser.newPage();
  const calls = [];
  await page.exposeFunction('hostCall', async (name, args) => {
    calls.push({ name, arguments: args });
    return answers[name] ?? { isError: true, content: [{ type: 'text', text: `no ${name}` }] };
  });
  const violations = [];
  page.on('console', (message) => {
    if (/Content Security Policy/.test(message.text())) violations.push(message.text());
  });
  await page.goto(`http://127.0.0.1:${server.address().port}/`);
  let sandbox;
  for (let tries = 0; tries < 100 && !sandbox; tries++) {
    sandbox = page.frames().find((f) => new URL(f.url()).pathname === '/sandbox');
    if (!sandbox) await new Promise((resolve) => setTimeout(resolve, 100));
  }
  await sandbox.waitForFunction(() => typeof window.show === 'function');
  await sandbox.evaluate((card) => window.show(card), { ...resource, context, result });
  const card = () => page.frames().find((frame) => frame.parentFrame() === sandbox);
  for (let tries = 0; tries < 100; tries++) {
    const frame = card();
    if (frame && (await frame.evaluate(() => !!document.querySelector('.tonk')).catch(() => false)))
      break;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  return {
    page,
    calls,
    violations,
    /** The card's own frame. */
    card,
    /** What the card asked of the host, in order. */
    asked: () => sandbox.evaluate(() => window.asked),
    /** Tell the card the host's theme changed. */
    theme: (theme) => sandbox.evaluate((theme) => window.changeTheme(theme), theme),
    async close() {
      await browser.close();
      await new Promise((resolve) => server.close(resolve));
      await new Promise((resolve) => fonts.close(resolve));
    },
  };
}
