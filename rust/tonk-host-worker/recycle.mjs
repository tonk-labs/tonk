// Fill one space writer past its high water and check it recycles cleanly.
// node recycle.mjs <token file> [records] [scans]
// The token file holds `{ "token", "space" }`. Run the worker under
// `wrangler dev` and watch its log for "space writer: recycling at".
// Every scan must find every record, before and after the recycle.
import { readFileSync } from "node:fs";
const WORKER = process.env.WORKER ?? "http://127.0.0.1:8789";
const { token, space } = JSON.parse(readFileSync(process.argv[2] ?? ".token", "utf8"));
const records = Number(process.argv[3] ?? 2000);
const scans = Number(process.argv[4] ?? 6);
const BATCH = 200;
const SCHEMA = `attribute!: &recycle-note-title
  description: Title
  the: recycle.note/title
  as: text
attribute!: &recycle-note-tag
  description: Tag
  the: recycle.note/tag
  as: text
concept!: &recycle-note
  description: A note
  with:
    title: recycle-note-title
    tag: recycle-note-tag
`;
let session;
let id = 0;
const rpc = async (method, params) => {
  const headers = { authorization: `Bearer ${token}`, "content-type": "application/json", accept: "application/json, text/event-stream" };
  if (session) headers["mcp-session-id"] = session;
  const response = await fetch(`${WORKER}/mcp`, { method: "POST", headers, body: JSON.stringify({ jsonrpc: "2.0", id: ++id, method, params }) });
  session ??= response.headers.get("mcp-session-id");
  const text = await response.text();
  if (response.status !== 200) throw new Error(`${method}: ${response.status}: ${text.slice(0, 300)}`);
  return JSON.parse(text);
};
const tool = async (name, document) => {
  const started = performance.now();
  const answer = await rpc("tools/call", { name, arguments: { space, document } });
  const ms = (performance.now() - started).toFixed(0);
  const result = answer.result ?? {};
  const text = (result.content ?? []).map((c) => c.text).join("");
  if (result.isError) throw new Error(`${name}: ${text.slice(0, 300)}`);
  return { text, ms };
};
await rpc("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "recycle", version: "0" } });
await tool("tonk_evaluate", SCHEMA);
const run = Date.now();
for (let start = 0; start < records; start += BATCH) {
  let document = "";
  for (let i = start; i < Math.min(start + BATCH, records); i++) {
    document += `recycle-note!:\n  this: urn:recycle:${run}:${i}\n  title: "note ${i} ${"x".repeat(200)}"\n  tag: "recycle-${run}"\n`;
  }
  const { ms } = await tool("tonk_evaluate", document);
  console.log(`wrote ${Math.min(start + BATCH, records)} of ${records} (${ms} ms)`);
}
let ok = true;
for (let scan = 0; scan < scans; scan++) {
  const { text, ms } = await tool("tonk_query", `recycle.note:\n  this: ?n\n  tag: "recycle-${run}"\n`);
  const found = new Set(text.match(new RegExp(`urn:recycle:${run}:\\d+`, "g")) ?? []).size;
  console.log(`scan ${scan}: ${found} of ${records} (${ms} ms)`);
  ok &&= found === records;
}
const { text } = await tool("tonk_evaluate", `recycle-note!:\n  this: urn:recycle:${run}:after\n  title: "after"\n  tag: "recycle-${run}-after"\n`);
console.log("write after:", text.includes("revision_after") ? "landed" : text.slice(0, 200));
console.log(ok ? "every scan found every record" : "A SCAN MISSED RECORDS");
process.exit(ok ? 0 : 1);
