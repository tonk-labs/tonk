// Peak wasm memory and cold timings per operation, each in a fresh isolate:
// restart `wrangler dev`, run the operation once cold and once warm.
//
//   ACCESS_SERVICE_URL=http://127.0.0.1:41001 WRANGLER=.../wrangler node fresh.mjs
import { spawn, execSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const WORKER = "http://127.0.0.1:8788";
const { token, space } = JSON.parse(readFileSync(`${here}/.token`, "utf8"));
const auth = { authorization: `Bearer ${token}` };

async function start() {
  try { execSync(`pkill -f "[w]rangler-dist/cli.js dev --port 8788"`); } catch {}
  await new Promise((r) => setTimeout(r, 1500));
  const began = performance.now();
  const child = spawn(process.env.WRANGLER, ["dev", "--port", "8788", "--ip", "127.0.0.1", "--var", `ACCESS_SERVICE_URL:${process.env.ACCESS_SERVICE_URL}`], { cwd: here, stdio: ["ignore", "pipe", "pipe"] });
  await new Promise((resolve) => {
    const onData = (chunk) => { if (String(chunk).includes("Ready on")) resolve(); };
    child.stdout.on("data", onData);
    child.stderr.on("data", onData);
  });
  return { child, ready: performance.now() - began };
}

async function call(method, path, body, headers = {}) {
  const began = performance.now();
  const response = await fetch(WORKER + path, { method, body, headers: { ...auth, ...headers } });
  const bytes = new Uint8Array(await response.arrayBuffer());
  const timing = JSON.parse(response.headers.get("x-spike-timing") ?? "null");
  return { status: response.status, wall: performance.now() - began, timing, size: bytes.length, text: new TextDecoder().decode(bytes.slice(0, 400)) };
}

const asset = new Uint8Array(4 * 1024 * 1024).map((_, i) => (i * 13) & 255);
let stored = null;
const ops = {
  "query-point": () => call("POST", `/${space}/api/query`, `spike-note:\n  this: urn:spike:note:4242\n  title: ?title\n`),
  "query-rank-scan": () => call("POST", `/${space}/api/query`, `spike.note:\n  this: ?n\n  rank: ?rank\n`),
  "query-tag": () => call("POST", `/${space}/api/query`, `spike.note:\n  this: ?n\n  tag: "tag-3"\n`),
  "query-full-scan": () => call("POST", `/${space}/api/query`, `spike-note:\n  title: ?title\n`),
  "evaluate-one": () => call("POST", `/${space}/api/evaluate`, `spike-note!:\n  this: urn:spike:note:fresh\n  title: "fresh ${Date.now()}"\n  body: "x"\n  rank: 1\n  tag: "fresh"\n`),
  "store-asset-4mib": async () => { asset[0] = Date.now() & 255; const r = await call("PUT", `/${space}/`, asset, { "content-type": "application/octet-stream" }); try { stored = JSON.parse(r.text).entity; } catch {} return r; },
  "read-asset-4mib": () => call("GET", `/${space}/asset/${encodeURIComponent(stored)}`),
};

const only = process.argv[2]?.split(",");
for (const [name, op] of Object.entries(ops)) {
  if (only && !only.includes(name)) continue;
  const { child, ready } = await start();
  const idle = await call("GET", "/");
  const cold = await op();
  const warm = await op();
  const mb = (r) => ((r.timing?.wasm_memory_after ?? 0) / 1048576).toFixed(1);
  console.log(`${name}: ready=${ready.toFixed(0)}ms idle_mem=${(JSON.parse(idle.text).wasm_memory / 1048576).toFixed(1)}MiB cold=${cold.status} ${cold.wall.toFixed(0)}ms mem=${mb(cold)}MiB warm=${warm.status} ${warm.wall.toFixed(0)}ms mem=${mb(warm)}MiB size=${warm.size}B phases=${JSON.stringify(cold.timing?.phases)}`);
  if (cold.status !== 200) console.log("  ", cold.text);
  child.kill();
}
try { execSync(`pkill -f "[w]rangler-dist/cli.js dev --port 8788"`); } catch {}
