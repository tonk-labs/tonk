// Parallel writes through the Host Worker's space writers, then a read back.
// node concurrent.mjs <n> <MCP session ids> <token files>, lists comma separated.
// Token files hold what the spike Worker's `POST /_spike/setup` answers.
import { readFileSync } from "node:fs";
const WORKER = process.env.WORKER ?? "http://127.0.0.1:8789";
const n = Number(process.argv[2] ?? 5);
const sessions = (process.argv[3] ?? "default").split(",");
const tokens = (process.argv[4] ?? ".token").split(",").map((f) => JSON.parse(readFileSync(f, "utf8")));
const { space } = tokens[0];
const SCHEMA = `attribute!: &spike-note-title
  description: Title
  the: spike.note/title
  as: text
attribute!: &spike-note-tag
  description: Tag
  the: spike.note/tag
  as: text
concept!: &spike-note
  description: A note
  with:
    title: spike-note-title
    tag: spike-note-tag
`;
const post = (path, body, i = 0) =>
  fetch(`${WORKER}/${space}/${path}`, {
    method: "POST",
    headers: { authorization: `Bearer ${tokens[i % tokens.length].token}`, "mcp-session-id": sessions[i % sessions.length] },
    body,
  });
const schema = await post("api/evaluate", SCHEMA);
console.log("schema:", schema.status, schema.status === 200 ? "" : await schema.text());
const run = Date.now();
const started = performance.now();
const results = await Promise.all(
  Array.from({ length: n }, (_, i) =>
    post("api/evaluate", `spike-note!:\n  this: urn:spike:concurrent:${run}:${i}\n  title: "concurrent ${i}"\n  tag: "concurrent-${run}"\n`, i).then(async (r) => {
      const text = await r.text();
      let revision = null;
      try { revision = JSON.parse(text).revision_after; } catch {}
      return { status: r.status, text: text.slice(0, 300), revision };
    }),
  ),
);
console.log(`writes (${(performance.now() - started).toFixed(0)} ms):`, results.map((r) => r.status).join(" "));
for (const r of results) {
  if (r.status !== 200) console.log("  ", r.text);
  else console.log(`   issuer=${r.revision?.issuer} edition=${r.revision?.edition}`);
}
const read = await post("api/query", `spike.note:\n  this: ?n\n  tag: "concurrent-${run}"\n`);
const body = await read.json();
const found = new Set(JSON.stringify(body.matches_after ?? body).match(/urn:spike:concurrent:\d+:\d+/g) ?? []).size;
console.log(`read back: ${found} of ${n}`);
