// Drive the spike Worker: set up, seed a synthetic space, exercise each
// operation and print the timings the Worker reports.
//
//   node drive.mjs setup [session]        -> prints the token JSON
//   node drive.mjs seed <records> [batch] -> writes <records> notes
//   node drive.mjs ops                    -> list/query/subscribe/evaluate/asset
//   node drive.mjs bench <n>              -> n queries, timings per phase
//
// WORKER (default http://127.0.0.1:8788) is the spike Worker; the token is
// kept in .token next to this file.
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const WORKER = process.env.WORKER ?? "http://127.0.0.1:8788";
const tokenFile = join(here, ".token");

async function call(method, path, body, headers = {}) {
  const started = performance.now();
  const response = await fetch(WORKER + path, { method, body, headers });
  const bytes = new Uint8Array(await response.arrayBuffer());
  const wall = performance.now() - started;
  let timing = null;
  try {
    timing = JSON.parse(response.headers.get("x-spike-timing"));
  } catch {}
  return { status: response.status, bytes, text: new TextDecoder().decode(bytes), wall, timing, headers: response.headers };
}

function auth() {
  const { token, space } = JSON.parse(readFileSync(tokenFile, "utf8"));
  return { headers: { authorization: `Bearer ${token}` }, space };
}

function summary(label, r) {
  const t = r.timing ?? {};
  const phases = Object.entries(t.phases ?? {}).map(([k, v]) => `${k}=${v.toFixed(0)}`).join(" ");
  const mem = t.wasm_memory_after ? ` mem=${(t.wasm_memory_after / 1048576).toFixed(1)}MiB` : "";
  console.log(`${label}: ${r.status} wall=${r.wall.toFixed(0)}ms ${phases}${mem}`);
}

const NOTE_SCHEMA = `attribute!: &spike-note-title
  description: Title
  the: spike.note/title
  as: text
attribute!: &spike-note-body
  description: Body
  the: spike.note/body
  as: text
attribute!: &spike-note-rank
  description: Rank
  the: spike.note/rank
  as: natural
attribute!: &spike-note-tag
  description: Tag
  the: spike.note/tag
  as: text
concept!: &spike-note
  description: A synthetic note
  with:
    title: spike-note-title
    body: spike-note-body
    rank: spike-note-rank
    tag: spike-note-tag
`;

function words(n, seed) {
  const vocabulary = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau upsilon phi chi psi omega".split(" ");
  let x = seed * 2654435761 >>> 0;
  const out = [];
  for (let i = 0; i < n; i++) {
    x = (x * 1103515245 + 12345) >>> 0;
    out.push(vocabulary[x % vocabulary.length]);
  }
  return out.join(" ");
}

function notes(from, count) {
  let doc = "";
  for (let i = from; i < from + count; i++) {
    doc += `spike-note!:
  this: urn:spike:note:${i}
  title: "Note ${i}: ${words(4, i)}"
  body: "${words(60, i + 7)}"
  rank: ${i}
  tag: "tag-${i % 50}"
`;
  }
  return doc;
}

const command = process.argv[2];
if (command === "setup") {
  const session = process.argv[3];
  const r = await call("POST", session ? `/_spike/setup?session=${session}` : "/_spike/setup");
  console.log(r.text);
  writeFileSync(session ? `${tokenFile}${session}` : tokenFile, r.text);
} else if (command === "seed") {
  const total = Number(process.argv[3] ?? 1000);
  const batch = Number(process.argv[4] ?? 200);
  const start = Number(process.argv[5] ?? 0);
  const { headers, space } = auth();
  if (start === 0) {
    const r = await call("POST", `/${space}/api/evaluate`, NOTE_SCHEMA, headers);
    summary("schema", r);
    if (r.status !== 200) { console.log(r.text.slice(0, 2000)); process.exit(1); }
  }
  for (let i = start; i < total; i += batch) {
    const r = await call("POST", `/${space}/api/evaluate`, notes(i, Math.min(batch, total - i)), headers);
    summary(`seed ${i}..${i + batch}`, r);
    if (r.status !== 200) { console.log(r.text.slice(0, 2000)); process.exit(1); }
  }
} else if (command === "ops") {
  const { headers, space } = auth();
  let r = await call("GET", "/spaces", undefined, headers);
  summary("list spaces", r); console.log("  ", r.text);
  r = await call("POST", `/${space}/api/query`, `spike-note:\n  this: urn:spike:note:7\n  title: ?title\n`, headers);
  summary("query one", r); console.log("  ", r.text.slice(0, 300));
  r = await call("POST", `/${space}/api/query`, `spike-note:\n  tag: "tag-3"\n  title: ?title\n`, headers);
  summary("query tag", r); console.log("  ", r.text.length, "bytes");
  r = await call("POST", `/${space}/api/evaluate`, `spike-note!:\n  this: urn:spike:note:ops\n  title: "Written at ${new Date().toISOString()}"\n  body: "ops"\n  rank: 0\n  tag: "ops"\n`, headers);
  summary("evaluate", r); console.log("  ", r.text.slice(0, 300));
  const wire = JSON.stringify({ predicate: { with: { title: { the: "spike.note/title", as: "Text" }, tag: { the: "spike.note/tag", as: "Text" } } }, terms: {} });
  r = await call("POST", `/${space}/api/subscribe?ms=1500`, wire, { ...headers, "content-type": "application/json" });
  summary("subscribe", r); console.log("  ", r.text.slice(0, 300));
  const asset = new Uint8Array(256 * 1024).map((_, i) => (i * 31) & 255);
  r = await call("PUT", `/${space}/`, asset, { ...headers, "content-type": "application/octet-stream" });
  summary("store asset", r); console.log("  ", r.text.slice(0, 300));
  let entity = null;
  try { entity = JSON.parse(r.text).entity ?? JSON.parse(r.text).ref ?? null; } catch {}
  if (entity) {
    r = await call("GET", `/${space}/asset/${encodeURIComponent(entity)}`, undefined, headers);
    summary("read asset", r);
    console.log("   bytes", r.bytes.length, "match", r.bytes.length === asset.length && r.bytes.every((b, i) => b === asset[i]));
  }
} else if (command === "bench") {
  const n = Number(process.argv[3] ?? 10);
  const { headers, space } = auth();
  for (let i = 0; i < n; i++) {
    const r = await call("POST", `/${space}/api/query`, `spike-note:\n  this: urn:spike:note:${i * 13}\n  title: ?title\n`, headers);
    summary(`query ${i}`, r);
  }
} else if (command === "cpu") {
  // CPU of the workerd process serving the Worker (WORKERD_PID), from
  // /proc, per operation: utime + stime, 10 ms ticks, averaged over n.
  const pid = process.env.WORKERD_PID;
  const n = Number(process.argv[3] ?? 20);
  const only = process.argv[4];
  const { headers, space } = auth();
  const ticks = () => {
    const fields = readFileSync(`/proc/${pid}/stat`, "utf8").split(") ")[1].split(" ");
    return Number(fields[11]) + Number(fields[12]);
  };
  const asset = new Uint8Array(1024 * 1024).map((_, i) => (i * 7) & 255);
  let stored = null;
  const ops = {
    "query-point": (i) => call("POST", `/${space}/api/query`, `spike-note:\n  this: urn:spike:note:${(i * 977) % 20000}\n  title: ?title\n`, headers),
    "query-tag": (i) => call("POST", `/${space}/api/query`, `spike-note:\n  tag: "tag-${i % 50}"\n  title: ?title\n`, headers),
    "query-scan": () => call("POST", `/${space}/api/query`, `spike-note:\n  title: ?title\n  rank: ?rank\n`, headers),
    "evaluate-one": (i) => call("POST", `/${space}/api/evaluate`, `spike-note!:\n  this: urn:spike:note:cpu-${i}\n  title: "cpu ${i} ${Date.now()}"\n  body: "x"\n  rank: ${i}\n  tag: "cpu"\n`, headers),
    "store-asset-1mib": async (i) => { asset[0] = i & 255; asset[1] = Date.now() & 255; const r = await call("PUT", `/${space}/`, asset, { ...headers, "content-type": "application/octet-stream" }); try { stored = JSON.parse(r.text).entity; } catch {} return r; },
    "read-asset-1mib": () => call("GET", `/${space}/asset/${encodeURIComponent(stored)}`, undefined, headers),
  };
  for (const [name, op] of Object.entries(ops)) {
    if (only && !only.split(",").includes(name)) continue;
    await op(0);
    const walls = [];
    let peak = 0;
    let statuses = new Set();
    const before = ticks();
    for (let i = 1; i <= n; i++) {
      const r = await op(i);
      walls.push(r.wall);
      statuses.add(r.status);
      peak = Math.max(peak, r.timing?.wasm_memory_after ?? 0);
      if (r.status !== 200) console.log(name, r.status, r.text.slice(0, 300));
    }
    const cpu = (ticks() - before) * 10 / n;
    walls.sort((a, b) => a - b);
    console.log(`${name}: n=${n} status=${[...statuses]} cpu=${cpu.toFixed(0)}ms/op wall p50=${walls[Math.floor(n / 2)].toFixed(0)} p90=${walls[Math.floor(n * 0.9)].toFixed(0)} max=${walls[n - 1].toFixed(0)}ms wasm_mem=${(peak / 1048576).toFixed(1)}MiB`);
  }
} else {
  console.log("usage: node drive.mjs setup|seed|ops|bench");
}
