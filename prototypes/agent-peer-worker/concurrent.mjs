// Concurrent writes against the same space through one isolate, then read
// them all back. `node concurrent.mjs <n>`.
import { readFileSync } from "node:fs";
import { dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
// Token files to spread the writes over, round robin: `.token` is session
// 0; `node drive.mjs setup <n>` writes `.token<n>` for session n.
const files = (process.argv[3] ?? ".token").split(",");
const tokens = files.map((file) => JSON.parse(readFileSync(`${here}/${file}`, "utf8")));
const { space } = tokens[0];
const headers = { authorization: `Bearer ${tokens[0].token}` };
const n = Number(process.argv[2] ?? 5);
const run = Date.now();
const results = await Promise.all(
  Array.from({ length: n }, (_, i) =>
    fetch(`http://127.0.0.1:8788/${space}/api/evaluate`, {
      method: "POST",
      headers: { authorization: `Bearer ${tokens[i % tokens.length].token}` },
      body: `spike-note!:\n  this: urn:spike:concurrent:${run}:${i}\n  title: "concurrent ${i}"\n  body: "x"\n  rank: ${i}\n  tag: "concurrent-${run}"\n`,
    }).then(async (r) => {
      const text = await r.text();
      let revision = null;
      try { revision = JSON.parse(text).revision_after; } catch {}
      return { status: r.status, text: text.slice(0, 200), revision };
    }),
  ),
);
console.log("writes:", results.map((r) => r.status).join(" "));
for (const r of results) {
  if (r.status !== 200) console.log("  ", r.text);
  else console.log(`   revision_after issuer=${r.revision?.issuer} edition=${r.revision?.edition}`);
}
const read = await fetch(`http://127.0.0.1:8788/${space}/api/query`, {
  method: "POST",
  headers,
  body: `spike.note:\n  this: ?n\n  tag: "concurrent-${run}"\n`,
});
const body = await read.json();
const found = JSON.stringify(body.matches_after ?? body).match(/urn:spike:concurrent:/g)?.length ?? 0;
console.log(`read back: ${found} of ${n}`);
