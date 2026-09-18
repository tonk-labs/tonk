// The whole path: a browser asks a `tonk` who it is, across the worker
// boundary, having exchanged nothing to find it.
//
//   page            derives port + fingerprint, dials, relays
//     |  MessagePort
//   "worker"        iroh endpoint over the transferred carrier
//     |  QUIC over SCTP over DTLS
//   tonk            verifies the invocation, answers `peer::Hello`
//
// The "worker" is `tonk-rtc-probe`, a wasm harness exposing the three
// calls `tonk-worker` makes. It covers the transport on its own, in
// seconds and without a `trunk build`; `cli-status-app.mjs` beside it
// runs the same path through the real page, service worker and route.
// Keep both: when the app test fails, this one says whether the
// transport or the wiring above it moved.
//
//   cargo build -p tonk-rtc-probe --target wasm32-unknown-unknown
//   wasm-bindgen --target web --out-dir <pkg> target/.../tonk_rtc_probe.wasm
//   node rust/tonk-ui/tests/e2e/cli-status.mjs

import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, "../../../..");
const CHROME = process.env.CHROME
  ?? "/opt/pw-browsers/chromium-1194/chrome-linux/chrome";
const PKG = process.env.PROBE_PKG ?? "/tmp/wcprobe/pkg";

const types = { ".js": "text/javascript", ".mjs": "text/javascript", ".wasm": "application/wasm" };
function origin() {
  const files = {
    "/": ["text/html", Buffer.from("<!doctype html><title>cli status</title>")],
    "/rtc.mjs": [types[".mjs"], readFileSync(path.join(repo, "rust/tonk-ui/assets/rtc.mjs"))],
    "/rendezvous.der": ["application/octet-stream",
      readFileSync(path.join(repo, "rust/tonk-rtc/assets/rendezvous.der"))],
    "/probe.js": [types[".js"], readFileSync(path.join(PKG, "tonk_rtc_probe.js"))],
    "/probe_bg.wasm": [types[".wasm"], readFileSync(path.join(PKG, "tonk_rtc_probe_bg.wasm"))],
  };
  const server = createServer((request, response) => {
    const entry = files[request.url.split("?")[0]];
    if (!entry) return response.writeHead(404).end();
    response.writeHead(200, { "content-type": entry[0] }).end(entry[1]);
  });
  return new Promise((resolve) => server.listen(0, "127.0.0.1", () => resolve(server)));
}

function listen() {
  const child = spawn(path.join(repo, "target/debug/examples/rendezvous_serve"), [], {
    stdio: ["ignore", "pipe", "inherit"],
  });
  const lines = [];
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
  return { child, until };
}

const server = await origin();
const { child, until } = listen();
let failure;

try {
  const up = await until("PEER ", 30);
  const peer = up.slice("PEER ".length).trim();
  console.log(`\nlistener is ${peer}\n`);

  const browser = await chromium.launch({ executablePath: CHROME });
  const page = await browser.newPage();
  page.on("console", (message) => console.log(`  page| ${message.text()}`));
  await page.goto(`http://127.0.0.1:${server.address().port}/`);

  const greeting = await page.evaluate(async (peer) => {
    const rtc = await import("/rtc.mjs");
    const probe = await import("/probe.js");
    await probe.default("/probe_bg.wasm");

    // The worker side: an endpoint over a transport with no carrier yet.
    const reach = await probe.Reach.bind();

    // The page side: dial, relay, hand the carrier over. `attachCarrier`
    // posts to a worker; here the "worker" is in this page, so the port
    // is handed to it directly instead.
    const address = await rtc.localAddress("/rendezvous.der");
    const { channel } = await rtc.dial(address, rtc.freshCredential(), rtc.datagramChannel());
    const { port1, port2 } = new MessageChannel();

    // Count both directions before relaying, so a stall says which way.
    let fromCli = 0;
    let toCli = 0;
    channel.addEventListener("message", () => { fromCli += 1; });
    port1.addEventListener("message", () => { toCli += 1; });

    rtc.relay(channel, port1);
    reach.attach(port2);

    try {
      const greeting = await reach.hello(peer);
      console.log(`datagrams: ${toCli} to the cli, ${fromCli} back`);
      return greeting;
    } catch (error) {
      console.log(`datagrams: ${toCli} to the cli, ${fromCli} back`);
      console.log(`channel is "${channel.readyState}"`);
      throw error;
    }
  }, peer);

  console.log(`\ngreeting: ${greeting}`);
  const [subject, profile, operator] = greeting.split(" ");
  for (const [name, did] of [["subject", subject], ["profile", profile], ["operator", operator]]) {
    if (!did?.startsWith("did:")) throw new Error(`${name} is not a DID: ${did}`);
  }

  console.log("\nPASS: a browser asked a tonk who it is, across the worker boundary.");
  await browser.close();
} catch (error) {
  failure = error;
  console.error(`\nFAIL: ${error.message}`);
} finally {
  child.kill();
  server.close();
}

process.exit(failure ? 1 : 0);
