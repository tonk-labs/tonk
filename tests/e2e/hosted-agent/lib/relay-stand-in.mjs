// A stand-in provider for the relay alone: what a shown view loads from
// the provider's origin, with no access service, worker or space behind
// it. With it, `showView` runs the real relay page in a real browser, so
// Tonk's line, loading and failure, display modes, links and theme are
// tested in seconds rather than against a deployment.
//
// It serves:
//   /in-agent/fonts/*   the fonts the cards ship (rust/tonk-ui/assets/in-agent)
//   /view-peer/*.js     a view peer that answers pull, status and the page's
//                       site from memory, instead of syncing a space
//   /space-runtime.json an empty runtime pack
//   /worker-guest       a guest page that talks the relay's guest bridge as
//                       `guest` scripts it: a title, a size, a link
// all with CORS, as the provider serves them.
//
// It cannot stand in for rendering a space: the guest draws no view.

import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const ROOT = fileURLToPath(new URL('../../../../', import.meta.url));
const HOSTED = `${ROOT}rust/tonk-worker/src/router/hosted/`;
const FONTS = `${ROOT}rust/tonk-ui/assets/in-agent/fonts/`;

// The relay as `view::read_resource` serves it: the in-agent stylesheet
// where its mark is, and the origin filled in. (`in_agent::splice` also
// adds font preloads; they change nothing a test reads.)
export async function relayResource(origin) {
  const css = (await readFile(`${HOSTED}in-agent/in-agent.css`, 'utf8')).replaceAll(
    '__TONK_IN_AGENT_FONTS__',
    `${origin}/in-agent/fonts`,
  );
  const html = (await readFile(`${HOSTED}view/relay.html`, 'utf8'))
    .replace('<!-- tonk:in-agent -->', () => `<style>\n${css}</style>`)
    .replace('__TONK_MCP_ORIGIN__', JSON.stringify(origin))
    .replace('__TONK_VIEW_FRAME__', 'false');
  const domains = [origin, origin.replace(/^http/, 'ws')];
  return {
    contents: [
      {
        text: html,
        _meta: { ui: { csp: { connectDomains: domains, resourceDomains: [origin] } } },
      },
    ],
  };
}

// The peer: `ViewKey` and `ViewPeer` as the wasm exports them, answering
// from `answers` (path suffix -> JSON) and failing whatever else.
const PEER = (answers) => `
export default async function init() {}
export class ViewKey {
  static async generate() { return new ViewKey(); }
  static async restore() { return new ViewKey(); }
  get did() { return 'did:key:z6MkStandInViewKey'; }
  async export() { return { stand: 'in' }; }
}
const answers = ${JSON.stringify(answers)};
export class ViewPeer {
  static async open() { return new ViewPeer(); }
  async grant() {}
  async refresh() {}
  follow() {}
  async fetch(request) {
    const path = new URL(request.url).pathname;
    for (const [suffix, answer] of Object.entries(answers))
      if (path.endsWith(suffix))
        return new Response(JSON.stringify(answer.body ?? {}), {
          status: answer.status ?? 200,
          headers: { 'content-type': 'application/json' },
        });
    return new Response('not stood in: ' + path, { status: 404 });
  }
}`;

// The guest: on load it opens the bridge and runs `script`, a list of
// steps, each a message for the relay. `{ wait: ms }` pauses. What the
// relay sends back is kept on `window.heard`.
const GUEST = (script) => `<!doctype html><meta charset="utf-8"><body>stand-in guest<script>
window.heard = [];
addEventListener('message', (e) => window.heard.push(e.data));
const steps = ${JSON.stringify(script)};
const channel = new MessageChannel();
channel.port1.onmessage = (e) => window.heard.push(e.data);
(async () => {
  for (const step of steps) {
    if (step.wait) await new Promise((r) => setTimeout(r, step.wait));
    else if (step.hello) parent.postMessage({ type: 'hello', v: 1 }, '*', [channel.port2]);
    else if (step.port) channel.port1.postMessage(step.port);
    else parent.postMessage(step, '*');
  }
})();
</script>`;

/**
 * Serve a stand-in provider on a free port. `peer` answers the view
 * peer's requests by path suffix; `guest` is the guest page's script
 * (omit it, or pass `null`, for a page that never speaks).
 */
export async function standIn({ peer = {}, guest = null } = {}) {
  const served = [];
  const server = createServer(async (request, response) => {
    const { pathname } = new URL(request.url, 'http://stand-in');
    served.push(pathname);
    const send = (status, type, body) => {
      response.writeHead(status, { 'content-type': type, 'access-control-allow-origin': '*' });
      response.end(body);
    };
    if (pathname.startsWith('/in-agent/fonts/')) {
      const name = pathname.slice('/in-agent/fonts/'.length);
      if (!/^[\w.-]+$/.test(name)) return send(404, 'text/plain', '');
      try {
        return send(200, 'font/woff2', await readFile(FONTS + name));
      } catch {
        return send(404, 'text/plain', '');
      }
    }
    if (pathname === '/view-peer/tonk_view_peer.js')
      return send(200, 'text/javascript', PEER(peer));
    if (pathname === '/view-peer/tonk_view_peer_bg.wasm') return send(200, 'application/wasm', '');
    if (pathname === '/space-runtime.json') return send(200, 'application/json', '{}');
    if (pathname === '/worker-guest')
      return send(200, 'text/html', guest ? GUEST(guest) : '<!doctype html><p>silent');
    send(404, 'text/plain', 'not stood in');
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  return {
    origin,
    served,
    close: () => new Promise((resolve) => server.close(resolve)),
  };
}

/** A client whose `tonk_view_access` answers `access`, as the MCP server would. */
export function standInClient(access) {
  const called = [];
  return {
    called,
    async callTool({ name, arguments: args }) {
      called.push({ name, args });
      if (name !== 'tonk_view_access')
        return { isError: true, content: [{ type: 'text', text: 'not stood in' }] };
      return access;
    },
  };
}
