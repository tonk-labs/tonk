// A stand-in MCP Apps host for a shown view: what Claude or ChatGPT does
// with `tonk_open_space`, driven by the test's own MCP client.
//
// Its shape follows the hosts': the chat page (on one origin) frames a
// sandbox page on another origin, which renders the view's HTML in a
// `srcdoc` frame under a Content-Security-Policy built from the
// resource's `_meta.ui.csp`. The sandbox answers the view's JSON-RPC
// (`ui/initialize`, then the tool result, then the view's own
// `tools/call`s, which go to the MCP server through `client`).
//
// What it cannot stand in for: the real hosts' policies beyond the
// declared domains, ChatGPT's `window.openai`, and whether a host honours
// `frameDomains` for a custom connector at all.

import { createServer } from 'node:http';
import { chromium } from 'playwright';

const CHAT = (sandbox) => `<!doctype html><meta charset="utf-8"><title>chat</title>
<iframe id="view" src="${sandbox}" sandbox="allow-scripts allow-same-origin allow-forms"
  style="width:960px;height:720px;border:0"></iframe>`;

const SANDBOX = `<!doctype html><meta charset="utf-8"><title>sandbox</title>
<style>html,body,iframe{margin:0;width:100%;height:100%;border:0}</style>
<iframe id="app" sandbox="allow-scripts allow-same-origin allow-forms"></iframe>
<script>
  const app = document.getElementById('app');
  let shown;
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
        hostContext: { theme: 'light', displayMode: 'inline' },
      });
    } else if (m.method === 'ui/notifications/initialized') {
      send({ method: 'ui/notifications/tool-result', params: shown.result });
    } else if (m.method === 'tools/call') {
      try {
        reply(await window.hostCall(m.params.name, m.params.arguments ?? {}));
      } catch (error) {
        send({ id: m.id, error: { code: -32603, message: String(error.message ?? error) } });
      }
    } else if (m.id !== undefined) {
      reply({});
    }
  });
  window.show = (view) => {
    shown = view;
    app.srcdoc = view.html;
  };
</script>`;

// The policy a host builds from `_meta.ui.csp`, after Claude's sandbox
// (`claudemcpcontent.com/mcp_apps`, see sw-probe/FINDINGS.md): scripts
// inline, `eval`, `blob:` and `data:`, nothing else reachable but the
// declared domains, and frames only `blob:`/`data:` unless the host
// grants `frameDomains`.
function policy(csp, frames) {
  const resources = (csp.resourceDomains ?? []).join(' ');
  return [
    "default-src 'none'",
    `script-src 'self' 'unsafe-inline' 'unsafe-eval' blob: data: ${resources}`,
    `style-src 'self' 'unsafe-inline' ${resources}`,
    `img-src 'self' data: blob: ${resources}`,
    `font-src 'self' data: ${resources}`,
    `connect-src 'self' ${(csp.connectDomains ?? []).join(' ')}`,
    `worker-src 'self' blob: data: ${resources}`,
    `frame-src 'self' blob: data: ${frames ? (csp.frameDomains ?? []).join(' ') : ''}`,
  ].join('; ');
}

/**
 * Show `structuredContent` of a `tonk_open_space` result, with the
 * relay resource `resource` (from `resources/read`), in a fresh
 * browser. With `frames`, the host declares the resource's
 * `frameDomains` to the view and lets its policy frame them (route A);
 * without, it declares none and frames nothing (route B).
 */
export async function showView({ client, result, resource, frames, headed = false }) {
  const [content] = resource.contents;
  const csp = { ...(content._meta?.ui?.csp ?? {}) };
  if (!frames) delete csp.frameDomains;
  const server = createServer((request, response) => {
    const { pathname } = new URL(request.url, 'http://host');
    if (pathname === '/sandbox') {
      response.writeHead(200, {
        'content-type': 'text/html',
        'content-security-policy': policy(csp, frames),
      });
      return response.end(SANDBOX);
    }
    response.writeHead(200, { 'content-type': 'text/html' });
    response.end(CHAT(`http://localhost:${server.address().port}/sandbox`));
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const browser = await chromium.launch({ headless: !headed });
  const page = await browser.newPage();
  const calls = [];
  await page.exposeFunction('hostCall', async (name, args) => {
    calls.push(name);
    return client.callTool({ name, arguments: args });
  });
  const violations = [];
  const log = [];
  page.on('console', (message) => {
    const text = message.text();
    if (/Content Security Policy/.test(text)) violations.push(text);
    log.push(`${message.type()}: ${text.slice(0, 500)}`);
  });
  page.on('pageerror', (error) => log.push(`pageerror: ${error.message}`));
  // What the view logged, for a failure to show.
  const tail = () => log.slice(-40).join('\n');
  await page.goto(`http://127.0.0.1:${server.address().port}/`);
  const sandbox = await sandboxFrame(page);
  await sandbox.evaluate((view) => window.show(view), {
    html: content.text,
    csp,
    result: { content: result.content, structuredContent: result.structuredContent },
  });
  return {
    page,
    calls,
    violations,
    /** The frames below the view's own, where the space renders. */
    frames: () => page.frames().filter((frame) => frame !== page.mainFrame()),
    /** Resolve once some frame of the view shows every one of `texts`. */
    async shows(texts, timeout = 90_000) {
      const deadline = Date.now() + timeout;
      let seen = '';
      while (Date.now() < deadline) {
        seen = await visibleText(page);
        if (texts.every((text) => seen.includes(text))) return seen;
        const error = await relayError(page);
        if (error) throw new Error(`the view failed: ${error}\n${tail()}`);
        await new Promise((resolve) => setTimeout(resolve, 500));
      }
      throw new Error(
        `the view never showed ${JSON.stringify(texts.filter((t) => !seen.includes(t)))}; ` +
          `it showed: ${seen.slice(0, 2000)}\n${tail()}`,
      );
    },
    async close() {
      await browser.close();
      await new Promise((resolve) => server.close(resolve));
    },
  };
}

async function sandboxFrame(page) {
  for (let tries = 0; tries < 100; tries++) {
    const frame = page.frames().find((f) => new URL(f.url()).pathname === '/sandbox');
    if (frame) {
      await frame.waitForFunction(() => typeof window.show === 'function');
      return frame;
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error('the sandbox frame never loaded');
}

// What the relay (or the provider's page) says went wrong, if it says.
async function relayError(page) {
  for (const frame of page.frames()) {
    const error = await frame
      .evaluate(() => document.querySelector('#error')?.textContent?.trim())
      .catch(() => '');
    if (error) return error;
  }
  return '';
}

// Every frame's rendered text, joined: the space renders in frames the
// view makes (a `srcdoc` guest, or the provider's page and its guest).
async function visibleText(page) {
  const texts = await Promise.all(
    page.frames().map((frame) => frame.evaluate(shadowText).catch(() => '')),
  );
  return texts.join('\n');
}

// A document's text, shadow roots included: the space's page renders in
// custom elements, whose text `innerText` does not reach.
function shadowText() {
  const parts = [];
  const walk = (node) => {
    if (node.nodeType === Node.TEXT_NODE) {
      const text = node.textContent.trim();
      if (text) parts.push(text);
      return;
    }
    if (node.nodeName === 'SCRIPT' || node.nodeName === 'STYLE') return;
    if (node.shadowRoot) walk(node.shadowRoot);
    for (const child of node.childNodes) walk(child);
  };
  if (document.body) walk(document.body);
  return parts.join('\n');
}
