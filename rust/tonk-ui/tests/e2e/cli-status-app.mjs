// The same path as `cli-status.mjs`, but through the real app: the page
// the user loads, the service worker it registers, and the
// `/api/cli/status` and `/api/cli/spaces` routes inside it. No harness stands in for either
// half — `cli-status.mjs` covers the transport with a purpose-built
// wasm shim, and this covers the wiring that shim replaced.
//
//   page (dist/index.html + rtc.mjs)
//     |  postMessage("tonk-rtc-carrier", [port])
//   service worker (dist/worker_bg.wasm -> router::cli)
//     |  QUIC over SCTP over DTLS
//   tonk (`tonk rtc serve`)
//
//   cd rust/tonk-ui && trunk build
//   cargo build -p tonk-rtc --features iroh --example rendezvous_serve
//   node rust/tonk-ui/tests/e2e/cli-status-app.mjs

import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { readFile, stat } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, "../../../..");
const DIST = process.env.DIST ?? path.join(repo, "rust/tonk-ui/dist");
const CHROME = process.env.CHROME
  ?? "/opt/pw-browsers/chromium-1194/chrome-linux/chrome";
const SERVE = process.env.SERVE
  ?? path.join(repo, "target/debug/examples/rendezvous_serve");

const types = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".mjs": "text/javascript",
  ".wasm": "application/wasm",
  ".json": "application/json",
  ".css": "text/css",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".der": "application/octet-stream",
  ".yaml": "text/yaml",
  ".woff2": "font/woff2",
};

// The dist as the app expects to be served: real MIME types, and the
// service worker at the root scope. Anything missing is a 404 rather
// than a fallback to index.html, so a wrong path shows up as a wrong
// path instead of as HTML that fails to parse.
function origin() {
  const server = createServer(async (request, response) => {
    const url = new URL(request.url, "http://127.0.0.1");
    const relative = url.pathname === "/" ? "/index.html" : url.pathname;
    const file = path.join(DIST, relative);
    if (!file.startsWith(DIST)) return response.writeHead(403).end();
    try {
      if (!(await stat(file)).isFile()) throw new Error("not a file");
      response.writeHead(200, {
        "content-type": types[path.extname(file)] ?? "application/octet-stream",
        "cache-control": "no-store",
        "service-worker-allowed": "/",
      }).end(await readFile(file));
    } catch {
      response.writeHead(404).end();
    }
  });
  return new Promise((resolve) => server.listen(0, "127.0.0.1", () => resolve(server)));
}

function listen() {
  const child = spawn(SERVE, [], { stdio: ["ignore", "pipe", "inherit"] });
  const lines = [];
  const offered = () => lines
    .filter((line) => line.startsWith("SPACE "))
    .map((line) => {
      const [, subject, name] = line.split(" ");
      return { subject, name: name === "-" ? null : name };
    });
  child.stdout.on("data", (chunk) => {
    const text = String(chunk);
    process.stdout.write(text.replace(/^/gm, "  tonk| "));
    lines.push(...text.split("\n").filter(Boolean));
  });
  const until = (prefix, seconds) => new Promise((resolve, reject) => {
    const deadline = Date.now() + seconds * 1000;
    const poll = setInterval(() => {
      const line = lines.find((l) => l.startsWith(prefix));
      if (line) { clearInterval(poll); resolve(line); }
      else if (Date.now() > deadline) {
        clearInterval(poll);
        reject(new Error(`no "${prefix}" within ${seconds}s; saw: ${lines.join(" | ") || "(nothing)"}`));
      }
    }, 100);
  });
  return { child, until, offered };
}

const server = await origin();
const { child, until, offered } = listen();
let failure;
let browser;

try {
  const peer = (await until("PEER ", 30)).slice("PEER ".length).trim();
  console.log(`\nlistener is ${peer}\n`);

  browser = await chromium.launch({ executablePath: CHROME });
  const context = await browser.newContext();
  const page = await context.newPage();
  page.on("console", (message) => console.log(`  page| ${message.text()}`));
  page.on("pageerror", (error) => console.log(`  page! ${error.message}`));

  await page.goto(`http://127.0.0.1:${server.address().port}/`, { waitUntil: "load" });

  // The app registers and claims the worker itself; wait for it to be
  // the controller rather than merely registered, because an
  // uncontrolled page's `fetch` never reaches the router.
  await page.waitForFunction(
    () => navigator.serviceWorker.controller !== null,
    null,
    { timeout: 120_000 },
  );
  console.log("  page| service worker is controlling");

  // Before the status probe: no page has dialed, so the route must say
  // so rather than fail. This is the ordinary state and worth pinning.
  const before = await page.evaluate(
    async (peer) => (await fetch(`/api/cli/status?peer=${encodeURIComponent(peer)}`)).json(),
    peer,
  );
  console.log(`  page| before a carrier: ${JSON.stringify(before)}`);
  if (before.reachable) throw new Error("reachable before any carrier was opened");

  // Dial, and hand the carrier to the worker exactly as the app would.
  await page.evaluate(async () => {
    const rtc = await import("/rtc.mjs");
    await rtc.attachCarrier(navigator.serviceWorker.controller);
    console.log("carrier handed to the service worker");
  });

  const status = await page.evaluate(
    async (peer) => (await fetch(`/api/cli/status?peer=${encodeURIComponent(peer)}`)).json(),
    peer,
  );

  console.log(`\nstatus: ${JSON.stringify(status, null, 2)}`);
  if (!status.reachable) throw new Error(`not reachable: ${status.detail}`);
  for (const name of ["subject", "profile", "operator"]) {
    if (!status[name]?.startsWith("did:")) {
      throw new Error(`${name} is not a DID: ${status[name]}`);
    }
  }
  // The greeting echoes the invocation's subject, so a genuine round
  // trip answers with this page's own profile — not the listener's
  // endpoint key, which names the route rather than the authority.
  // Nothing in the worker can produce this locally: the only path to a
  // `Greeting` is the iroh site, over the carrier.
  const { did } = await page.evaluate(async () => (await fetch("/api/identify")).json());
  if (status.subject !== did) {
    throw new Error(`answered for ${status.subject}, but this page is ${did}`);
  }
  console.log(`\nthe tonk echoed this page's own subject: ${did}`);

  // What it holds, over the same carrier. A directory listing: every
  // space is a DID the page has no authority over and has not opened.
  const inventory = await page.evaluate(
    async (peer) => (await fetch(`/api/cli/spaces?peer=${encodeURIComponent(peer)}`)).json(),
    peer,
  );
  console.log(`\nspaces: ${JSON.stringify(inventory, null, 2)}`);
  if (!inventory.reachable) throw new Error(`not reachable: ${inventory.detail}`);

  // Against what the listener said it seeded, so this fails if the
  // values are lost or reordered on the way rather than merely if the
  // call succeeds.
  const expected = offered();
  if (expected.length === 0) throw new Error("the listener seeded no spaces to check against");
  const seen = (inventory.spaces ?? []).map((s) => `${s.subject} ${s.name ?? "-"}`).sort();
  const want = expected.map((s) => `${s.subject} ${s.name ?? "-"}`).sort();
  if (seen.join(" | ") !== want.join(" | ")) {
    throw new Error(`offered [${want}] but the page saw [${seen}]`);
  }
  console.log(`the tonk's ${seen.length} spaces arrived intact, names and all`);

  console.log("\nPASS: the real page asked a tonk who it is and what it holds, through the real service worker.");
} catch (error) {
  failure = error;
  console.error(`\nFAIL: ${error.message}`);
} finally {
  await browser?.close();
  child.kill();
  server.close();
}

process.exit(failure ? 1 : 0);
