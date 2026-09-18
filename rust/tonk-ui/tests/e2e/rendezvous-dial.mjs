// A real browser dialling a real listener, with nothing exchanged.
//
// This is the measurement everything else rests on. Two of the three
// things a dialer needs are derived here (the port from the phrase, the
// candidate from loopback) and the third — the DTLS fingerprint — is the
// hash of a certificate this origin serves. If Chromium completes the
// handshake against it, the "no exchange" property is real rather than
// argued for.
//
// Not part of `node --test 'rust/tonk-ui/tests/*.test.mjs'`: it needs a
// browser and a built binary, neither of which the unit pass has.
//
//   cargo build -p tonk-rtc --example rendezvous_listener
//   npm i playwright        # or set PLAYWRIGHT_NODE_MODULES
//   node rust/tonk-ui/tests/e2e/rendezvous-dial.mjs
//
// `CHROME` points at the browser; it defaults to the one this
// repository's dev container ships.

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

/** Serve rtc.mjs and the certificate from one origin, as the app does. */
function origin() {
  const files = {
    "/": ["text/html", Buffer.from("<!doctype html><title>dial</title>")],
    "/rtc.mjs": ["text/javascript", readFileSync(path.join(repo, "rust/tonk-ui/assets/rtc.mjs"))],
    "/rendezvous.der": ["application/octet-stream",
      readFileSync(path.join(repo, "rust/tonk-rtc/assets/rendezvous.der"))],
  };
  const server = createServer((request, response) => {
    const entry = files[request.url.split("?")[0]];
    if (!entry) return response.writeHead(404).end();
    response.writeHead(200, { "content-type": entry[0] }).end(entry[1]);
  });
  return new Promise((resolve) => server.listen(0, "127.0.0.1", () => resolve(server)));
}

/** Start the listener and wait for it to say it is up. */
function listen() {
  const child = spawn(path.join(repo, "target/debug/examples/rendezvous_listener"), [], {
    stdio: ["ignore", "pipe", "inherit"],
  });
  const lines = [];
  child.stdout.on("data", (chunk) => lines.push(...String(chunk).split("\n").filter(Boolean)));

  const until = (prefix, seconds) => new Promise((resolve, reject) => {
    const deadline = Date.now() + seconds * 1000;
    const poll = setInterval(() => {
      const line = lines.find((l) => l.startsWith(prefix));
      if (line) { clearInterval(poll); resolve(line); }
      else if (Date.now() > deadline) { clearInterval(poll); reject(new Error(`no "${prefix}" within ${seconds}s; saw: ${lines.join(" | ") || "(nothing)"}`)); }
    }, 100);
  });
  return { child, until };
}

const server = await origin();
const { child, until } = listen();
let failure;

try {
  const up = await until("LISTENING", 20);
  console.log(`listener: ${up}`);

  const browser = await chromium.launch({ executablePath: CHROME });
  const page = await browser.newPage();
  await page.goto(`http://127.0.0.1:${server.address().port}/`);

  const dialled = await page.evaluate(async () => {
    const m = await import("/rtc.mjs");
    const address = await m.localAddress("/rendezvous.der");
    const { channel } = await m.dial(address, m.freshCredential(), m.datagramChannel());
    return {
      port: address.candidates[0].port,
      fingerprint: address.fingerprint,
      label: channel.label,
      state: channel.readyState,
      ordered: channel.ordered,
    };
  });
  console.log("browser:", JSON.stringify(dialled));

  const accepted = await until("ACCEPTED", 20);
  console.log(`listener: ${accepted}`);

  if (dialled.state !== "open") throw new Error(`channel is "${dialled.state}"`);
  if (dialled.label !== "tonk-iroh") throw new Error(`label is "${dialled.label}"`);
  if (dialled.ordered !== false) throw new Error("channel is ordered; QUIC would fight it");
  if (!accepted.includes("label=tonk-iroh")) throw new Error(`listener saw ${accepted}`);

  console.log("\nPASS: a browser dialled a listener with nothing exchanged.");
  await browser.close();
} catch (error) {
  failure = error;
  console.error(`\nFAIL: ${error.message}`);
} finally {
  child.kill();
  server.close();
}

process.exit(failure ? 1 : 0);
