// The library's console elements, <space-tree> (tree.yaml) and
// <space-inspector> (inspector.yaml), driven in a real Chromium the way a
// space renders them: each definition read out of its YAML (every
// `!include/text` inlined), handed to the real element runtime, and the
// host answered by a stand-in that serves the worker's `tree/*` formula
// rows, evaluate responses and a language server behind `fetch`.
//
// Run it (after `npm run build` here):
//
//   node rust/tonk-code/tests/console-elements.mjs
//
// Playwright and Chromium come from the paths the dev image provides,
// overridable with PLAYWRIGHT_ROOT / CHROMIUM_BIN.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const library = join(here, "../../tonk-core/assets/library");
const RUNTIME = readFileSync(join(here, "../../tonk-display/assets/element-runtime.js"), "utf8");

const require = createRequire(import.meta.url);
const { chromium } = process.env.PLAYWRIGHT_ROOT
  ? await import(join(process.env.PLAYWRIGHT_ROOT, "index.mjs"))
  : require("playwright");

/** The `element!:` named `tag` in `file`, as the registry hands it over. */
function definition(file, tag) {
  const lines = readFileSync(join(library, file), "utf8").split("\n");
  const start = lines.findIndex((line) => line === `element!: &${tag}`);
  if (start === -1) throw new Error(`${file} declares no element ${tag}`);
  const out = { method: {}, attribute: {}, getter: {}, setter: {} };
  let dictionary = null;
  for (let i = start + 1; i < lines.length; i++) {
    const line = lines[i];
    if (/^\S/.test(line)) break;
    const section = line.match(/^  (method|attribute|getter|setter):\s*$/);
    if (section) { dictionary = out[section[1]]; continue; }
    const entry = line.match(/^    ([a-z][a-z0-9-]*):\s*(.*)$/);
    if (!entry || !dictionary) continue;
    const [, key, rest] = entry;
    const include = rest.match(/^!include\/text\s+(\S+)$/);
    if (include) {
      dictionary[key] = readFileSync(join(library, include[1]), "utf8");
    } else if (rest === "|") {
      const body = [];
      while (i + 1 < lines.length && (lines[i + 1].startsWith("      ") || lines[i + 1].trim() === "")) {
        body.push(lines[++i].slice(6));
      }
      dictionary[key] = body.join("\n");
    } else {
      dictionary[key] = rest.replace(/^"|"$/g, "");
    }
  }
  return out;
}

/** The module the registry executes for `tag` (`element_source.rs`). */
const module = (tag, { method, attribute, getter, setter }) => {
  const expressions = (map) =>
    "{\n" + Object.entries(map).map(([k, v]) => `  ${JSON.stringify(k)}: (${v.trimEnd()}),\n`).join("") + "}";
  return `globalThis.defineTonkElement?.(${JSON.stringify(tag)}, ${expressions(method)}, ${JSON.stringify(attribute)}, ${expressions(getter)}, ${expressions(setter)});\n`;
};

const results = [];
const check = (name, ok, detail = "") => results.push({ name, ok, detail: ok ? "" : detail });

const browser = await chromium.launch({ args: ["--no-proxy-server"] });
const problems = [];
const open = async () => {
  const page = await browser.newPage();
  page.on("console", (m) => { if (m.type() === "error") problems.push(m.text()); });
  page.on("pageerror", (e) => problems.push(String(e)));
  await page.setContent("<!doctype html><html><head></head><body></body></html>");
  await page.addScriptTag({ content: RUNTIME });
  return page;
};
const define = async (page, file, tag) => {
  await page.addScriptTag({ type: "module", content: module(tag, definition(file, tag)) });
  await page.waitForFunction((tag) => customElements.get(tag), tag);
};
const until = (page, predicate, arg) =>
  page.waitForFunction(predicate, arg, { timeout: 10_000 }).then(() => true, () => false);

// ---------------------------------------------------------------------------
// <space-tree>
// ---------------------------------------------------------------------------

const tree = await open();
await tree.evaluate(() => {
  // A root index with two children: a cached segment and one the replica
  // has not fetched. Hashes, keys and entries as the worker shapes them.
  const part = (kind, text, hex) => ({ kind, text, hex });
  const rows = {
    root: { this: "#root", fields: { kind: "index", size: 2048, count: 2, cached: true, scale: 3, novelty: 1,
      manifest: { "max-segment": 6, fanout: 16, version: 2 } } },
    children: [
      { this: "#root", fields: { child: "#seg", kind: "segment", size: 512, count: 3, cached: true, pending: 2,
        bound: "0x006162", "bound-parts": [part("index", "entity", "0x00"), part("entity", "ab", "0x6162")] } },
      { this: "#root", fields: { child: "#far", kind: "index", size: 0, count: 0, cached: false,
        bound: "0x006163", "bound-parts": [part("index", "entity", "0x00"), part("entity", "ac", "0x6163")] } },
    ],
    // Read again after a fetch, a block knows nothing of its separator.
    far: { this: "#far", fields: { kind: "segment", size: 100, count: 1, cached: true } },
    seg: { this: "#seg", fields: { kind: "segment", size: 512, count: 3, cached: true } },
    entries: [
      { this: "#seg", fields: { ordering: "entity", entity: "did:key:zAlice", attribute: "person/name", type: "text:",
        value: "Alice", "key-parts": [part("index", "entity", "0x00")] } },
      { this: "#seg", fields: { ordering: "history", entity: "did:key:zAlice", attribute: "person/name", type: "text:",
        value: "Al", origin: `0x${"ab".repeat(32)}`, edition: 7, cause: 1, "key-parts": [part("index", "history", "0x03")] } },
      { this: "#seg", fields: { ordering: "coverage", entity: "did:key:zAlice", attribute: "person/name", supersedes: 2,
        spill: "#spilled0123456789", type: "text:", "key-parts": [part("index", "coverage", "0x05")] } },
    ],
  };
  const asked = (globalThis.__asked = []);
  document.addEventListener("tonk-query", (event) => {
    event.preventDefault();
    const { predicate, terms } = event.detail.query;
    asked.push({ predicate, terms, with: event.composedPath()[0].getAttribute("with"), space: event.detail.space ?? null });
    const answer = predicate === "tree/node"
      ? (terms.hash === "#far" ? [rows.far] : terms.hash === "#seg" ? [rows.seg] : [rows.root])
      : predicate === "tree/child" ? (terms.hash === "#root" ? rows.children : [])
      : predicate === "tree/entry" ? rows.entries : [];
    event.detail.result = Promise.resolve(answer);
  });
});
await define(tree, "tree.yaml", "space-tree");
await tree.evaluate(() => {
  const el = document.createElement("space-tree");
  el.setAttribute("with", "main@did:key:zSpace");
  document.body.append(el);
});
const drawn = await until(tree, () => document.querySelector("space-tree").shadowRoot?.querySelector(".inspector h2"));
const root = await tree.evaluate(() => {
  const shadow = document.querySelector("space-tree").shadowRoot;
  const kv = Object.fromEntries([...shadow.querySelectorAll(".inspector .kv")].map((row) => [row.children[0].textContent, row.children[1].textContent]));
  return {
    title: shadow.querySelector(".inspector h2")?.textContent,
    kv,
    manifest: [...shadow.querySelectorAll(".inspector h3 ~ .kv .k")].map((k) => k.textContent),
    item: shadow.querySelector("wa-tree > wa-tree-item")?.getAttribute("data-hash"),
    lazy: shadow.querySelector("wa-tree > wa-tree-item")?.hasAttribute("lazy"),
    novelty: shadow.querySelector("wa-tree > wa-tree-item .nov-own")?.textContent,
    asked: globalThis.__asked[0],
  };
});
check("draws the root selected, with its detail", drawn && root.title === "Index node" && root.kv.hash === "root"
  && root.kv.children === "2" && root.kv.scale === "3" && root.kv.novelty === "1" && root.kv.storage === "local",
  JSON.stringify(root));
check("reads the manifest in its own order", root.manifest.join(",") === "version,fanout,max-segment", JSON.stringify(root.manifest));
check("routes its reads by its own with", root.asked?.predicate === "tree/node" && root.asked?.with === "main@did:key:zSpace"
  && root.asked?.space === null, JSON.stringify(root.asked));
check("an index with children expands lazily, with its novelty", root.item === "#root" && root.lazy && root.novelty === "⊕1", JSON.stringify(root));

const expanded = await tree.evaluate(async () => {
  const shadow = document.querySelector("space-tree").shadowRoot;
  const item = shadow.querySelector("wa-tree > wa-tree-item");
  item.dispatchEvent(new CustomEvent("wa-lazy-load"));
  for (let i = 0; i < 40 && !item.querySelector('wa-tree-item[data-hash="#far"]'); i++) await new Promise((r) => setTimeout(r, 25));
  await new Promise((r) => setTimeout(r, 50));
  const far = item.querySelector('wa-tree-item[data-hash="#far"]');
  const seg = item.querySelector('wa-tree-item[data-hash="#seg"]');
  return {
    children: [...item.querySelectorAll(":scope > wa-tree-item")].map((child) => child.getAttribute("data-hash")),
    rootLazy: item.hasAttribute("lazy"),
    farRemote: far?.querySelector(":scope > .row")?.classList.contains("remote"),
    farHollow: far?.querySelector(':scope > [slot="expand-icon"]')?.classList.contains("dot-remote"),
    farCloud: !!far?.querySelector(".remote-icon"),
    segLeaf: !!seg?.querySelector(":scope > .dot-leaf"),
    segPending: seg?.querySelector(".nov-pending")?.textContent,
    segChips: [...(seg?.querySelectorAll(".keystr .key-seg") ?? [])].map((chip) => [chip.textContent, chip.className]),
    tips: [...(seg?.querySelectorAll(".keystr wa-tooltip") ?? [])].map((tip) => tip.textContent),
  };
});
check("expanding lists the children in order", expanded.children.join(",") === "#seg,#far" && !expanded.rootLazy, JSON.stringify(expanded));
check("a node the replica lacks shows hollow, dimmed and clouded", expanded.farRemote && expanded.farHollow && expanded.farCloud, JSON.stringify(expanded));
check("a cached segment is a leaf with what its parent buffers for it", expanded.segLeaf && expanded.segPending === "↓2", JSON.stringify(expanded));
check("a child's key is chips named by their parts", expanded.segChips.map(([text]) => text).join("|") === "E|ab"
  && expanded.tips.join("|") === "Entity index|Entity", JSON.stringify(expanded));

const fetched = await tree.evaluate(async () => {
  const shadow = document.querySelector("space-tree").shadowRoot;
  const far = shadow.querySelector('wa-tree-item[data-hash="#far"]');
  far.dispatchEvent(new CustomEvent("wa-lazy-load"));
  for (let i = 0; i < 40 && far.querySelector(":scope > .row.remote"); i++) await new Promise((r) => setTimeout(r, 25));
  return {
    remote: !!far.querySelector(":scope > .row.remote"),
    filled: [...far.querySelectorAll(":scope > .dot")].every((dot) => dot.classList.contains("dot-local")),
    lazy: far.hasAttribute("lazy"),
    key: far.querySelector(":scope > .row .keystr")?.textContent,
  };
});
check("fetching a node redraws its row, keeping the separator its parent gave", !fetched.remote && fetched.filled && !fetched.lazy
  && fetched.key.includes("ac"), JSON.stringify(fetched));

const segment = await tree.evaluate(async () => {
  const shadow = document.querySelector("space-tree").shadowRoot;
  const seg = shadow.querySelector('wa-tree-item[data-hash="#seg"]');
  const event = new CustomEvent("wa-selection-change", { detail: { selection: [seg] } });
  shadow.querySelector("wa-tree").dispatchEvent(event);
  for (let i = 0; i < 40 && !shadow.querySelector(".entries table"); i++) await new Promise((r) => setTimeout(r, 25));
  const rows = [...shadow.querySelectorAll(".entries tr.entry")];
  const cells = rows.map((row) => [...row.children].map((cell) => cell.textContent));
  const detail = shadow.querySelectorAll(".entries tr.detail")[1];
  const hiddenBefore = detail.hidden;
  rows[1].click();
  return {
    title: shadow.querySelector(".inspector h2")?.textContent,
    count: shadow.querySelector(".entries > .k")?.textContent,
    cells,
    version: rows[1].querySelector(".version")?.title,
    unfolded: hiddenBefore && !detail.hidden,
    detail: Object.fromEntries([...detail.querySelectorAll(".kv")].map((row) => [row.children[0].textContent, row.children[1].textContent])),
    separator: [...shadow.querySelectorAll(".inspector > .keybytes .key-seg")].map((chip) => chip.textContent),
  };
});
check("selecting a segment lists its entries", segment.title === "Segment node" && segment.count === "3 entries", JSON.stringify(segment));
check("an entry reads index, entity, attribute and its value typed",
  JSON.stringify(segment.cells[0]) === JSON.stringify(["entity", "did:key:zAlice", "person/name", '"Alice"']), JSON.stringify(segment.cells));
check("a history record shows the revision that wrote it",
  segment.cells[1][0] === `history${"ab".repeat(4)}@7` && segment.version === `revision origin:${"ab".repeat(32)} @7`, JSON.stringify(segment.cells));
check("a coverage record says what it covers and where its value went",
  segment.cells[2][3] === "covers 2 · → text: spilled012", JSON.stringify(segment.cells));
check("an entry unfolds into its version and lineage", segment.unfolded && segment.detail.edition === "7"
  && segment.detail.cause === "1" && segment.detail.origin === "ab".repeat(32), JSON.stringify(segment.detail));
check("the selected node's separator shows whole", segment.separator.join("|") === "E|ab", JSON.stringify(segment.separator));

const pure = await tree.evaluate(() => {
  const el = document.querySelector("space-tree");
  const part = (kind, text, hex) => ({ kind, text, hex });
  const index = ["entity", "history", "blob", "coverage"].map((name) => {
    const [component] = el.components([part("index", name, "0x00")]);
    return [component.label, component.text];
  });
  const vtype = el.components([part("vtype", "String", "0x03")])[0].label;
  const spans = el.components([part("index", "entity", "0x00"), part("entity", "ab", "0x6162"), part("vtype", "String", "0x03")])
    .map((component) => component.bytes.join(".."));
  const history = el.components([
    part("index", "history", "0x03"),
    part("origin", "origin:abab", `0x${"ab".repeat(32)}`),
    part("edition", "@7", "0x0000000000000007"),
    part("entity", "test:subject", "0x746573743a7375626a656374"),
    part("attribute", "test/name", "0x746573742f6e616d65"),
  ]);
  return {
    index, vtype, spans,
    history: history.map((component) => component.label),
    colours: [history[3].className, history[4].className],
    glyph: el.components([part("entity", "a\u0000b\u0001", "0x61006201")])[0].text,
    pivot: [
      el.pivot("0x0102", null, null),
      el.pivot("0x000a", "0x0009", "0x000b"),
      el.pivot("0x0a0b0c", "0x0a0000", "0x0a0b0d"),
      el.pivot("0x01", "xyz", null),
    ],
    format: [
      el.format("Alice", "text:"),
      el.format(3, "float:"),
      el.format(2.5, "Float"),
      el.format(4, "float:32"),
      el.format(-3, "integer:"),
      el.format([1, 255], "bytes:"),
      el.format("did:key:z", "entity:"),
      el.format(true, "boolean:"),
    ],
  };
});
check("the index chip is named by its ordering and keeps one letter",
  JSON.stringify(pure.index) === JSON.stringify([["Entity index", "E"], ["History index", "H"], ["Blob index", "B"], ["Coverage index", "C"]]),
  JSON.stringify(pure.index));
check("the value-type chip is named by its type", pure.vtype === "Value type: String", pure.vtype);
check("each part spans its raw bytes", pure.spans.join(",") === "0..1,1..3,3..4", JSON.stringify(pure.spans));
check("a history key labels its version and keeps its fact's colours",
  pure.history.join("|") === "History index|Revision origin|Revision edition|Entity|Attribute"
    && pure.colours.join(",") === "seg-entity,seg-attribute", JSON.stringify(pure));
check("control bytes in a part show as glyphs", pure.glyph === "a␀b·", pure.glyph);
check("the pivot is where a key parts from its nearer neighbour", JSON.stringify(pure.pivot) === JSON.stringify([0, 1, 2, null]), JSON.stringify(pure.pivot));
check("a value's type reads from its spelling",
  JSON.stringify(pure.format) === JSON.stringify(['"Alice"', "3.0", "2.5", "4.0", "-3", "01 ff", "did:key:z", "true"]), JSON.stringify(pure.format));

const unrouted = await tree.evaluate(async () => {
  const el = document.createElement("space-tree");
  document.body.append(el);
  await new Promise((r) => setTimeout(r, 50));
  const before = el.shadowRoot.querySelector(".err")?.textContent;
  el.setAttribute("with", "main@profile:tonk");
  for (let i = 0; i < 40 && !el.shadowRoot.querySelector(".inspector h2"); i++) await new Promise((r) => setTimeout(r, 25));
  return { before, after: el.shadowRoot.querySelector(".inspector h2")?.textContent, with: globalThis.__asked.at(-1)?.with };
});
check("without a route it says so, and draws once one arrives",
  unrouted.before?.startsWith("no repository in context") && unrouted.after === "Index node" && unrouted.with === "main@profile:tonk",
  JSON.stringify(unrouted));

const explicit = await tree.evaluate(async () => {
  const el = document.createElement("space-tree");
  el.setAttribute("repo", "did:key:zOther");
  el.setAttribute("branch", "draft");
  document.body.append(el);
  for (let i = 0; i < 40 && !el.shadowRoot?.querySelector(".inspector h2"); i++) await new Promise((r) => setTimeout(r, 25));
  return globalThis.__asked.at(-1);
});
check("a repo attribute routes its reads explicitly", explicit.space === "did:key:zOther", JSON.stringify(explicit));

// ---------------------------------------------------------------------------
// <space-inspector>
// ---------------------------------------------------------------------------

const inspector = await open();
await inspector.evaluate(() => {
  // The language server: an error wherever a document says `bad`.
  const encoder = new TextEncoder();
  const streams = new Set();
  const push = (message) => {
    for (const controller of streams) controller.enqueue(encoder.encode(`data: ${JSON.stringify(message)}\n\n`));
  };
  const verdict = (uri, version, text) => push({
    jsonrpc: "2.0",
    method: "textDocument/publishDiagnostics",
    params: { uri, version, diagnostics: text.includes("bad")
      ? [{ range: { start: { line: 0, character: 0 }, end: { line: 0, character: 3 } }, severity: 1, message: "bad" }]
      : [] },
  });
  const fetched = (globalThis.__fetched = []);
  globalThis.__sent = [];
  window.fetch = async (url, init = {}) => {
    fetched.push(url);
    if (url.endsWith("/sync/status")) {
      return new Response(JSON.stringify({ state: "behind", local: { tree: "#local-head" }, remote: { tree: "#remote-head" } }));
    }
    if (url.startsWith("/api/repository/")) {
      return new Response(JSON.stringify({
        name: "did:key:zSpace", label: "Space", subject: "did:key:zSpace", operator: "did:key:zOperator",
        profile: "did:key:zProfile",
        branch: { main: { revision: { tree: [0, 1, 2, 3] }, upstream: { remote: "origin", branch: "main" } } },
        remote: { origin: { address: { Ucan: { endpoint: "https://access.example.test/ucan/" } } } },
      }));
    }
    if (url !== "/api/language-server") return new Response("", { status: 404 });
    if (init.method !== "POST") {
      return new Response(new ReadableStream({ start: (controller) => streams.add(controller) }), {
        headers: { "content-type": "text/event-stream" },
      });
    }
    const message = JSON.parse(init.body);
    globalThis.__sent.push(message);
    if (message.method === "initialize") {
      return new Response(JSON.stringify({ jsonrpc: "2.0", id: message.id, result: { capabilities: { textDocumentSync: 1 } } }));
    }
    if (message.method === "textDocument/didOpen") {
      const { uri, version, text } = message.params.textDocument;
      setTimeout(() => verdict(uri, version, text), 0);
    }
    if (message.method === "textDocument/didChange") {
      const { uri, version } = message.params.textDocument;
      setTimeout(() => verdict(uri, version, message.params.contentChanges.at(-1).text), 0);
    }
    return new Response("");
  };

  // The host: evaluations answer from `__answer`, and whether the space
  // has code-editor from `__installed`.
  globalThis.__evaluations = [];
  globalThis.__installed = false;
  globalThis.__answer = () => ({ revision_after: { tree: "#after" }, matches_before: [], matches_after: [] });
  const watchers = (globalThis.__watchers = new Set());
  document.addEventListener("tonk-evaluate", (event) => {
    event.preventDefault();
    const { document: notation, transact } = event.detail;
    globalThis.__evaluations.push({ notation, transact, with: event.composedPath()[0].getAttribute("with") });
    event.detail.result = Promise.resolve().then(() => globalThis.__answer(notation, transact));
  });
  document.addEventListener("tonk-query", (event) => {
    event.preventDefault();
    const named = event.detail.query?.terms?.this === "id:code-editor";
    event.detail.result = Promise.resolve(named && globalThis.__installed ? [{ this: "id:code-editor", fields: { entity: "element:x" } }] : []);
  });
  document.addEventListener("tonk-subscribe", (event) => {
    event.preventDefault();
    const consumer = event.composedPath()[0];
    watchers.add(consumer);
    event.detail.subscription = { cancel: () => watchers.delete(consumer) };
  });
});
await define(inspector, "inspector.yaml", "space-inspector");

const rendered = await inspector.evaluate(() => {
  const el = document.createElement("space-inspector");
  const result = (json) => el.result(null, json);
  return {
    failure: el.result("boom <bad>", null),
    empty: result({ revision_after: { tree: "#abcdefgh12345" }, matches_before: [], matches_after: [] }),
    bytes: result({ revision_after: { tree: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9] }, matches_before: [], matches_after: [] }),
    generic: result({
      matches_before: [{ label: "person", results: [{ this: "id:alice", fields: { name: "Alice" } }] }],
      matches_after: [{ label: "person", results: [{ this: "id:alice", fields: { name: "Alice" } }] }],
    }),
    changed: result({
      revision_before: { tree: "#before00" }, revision_after: { tree: "#after000" },
      matches_before: [], matches_after: [{ label: "person", results: [{ this: "id:bob", fields: {} }] }],
    }),
    concept: result({
      matches_before: [], matches_after: [{ label: "concept", results: [{ this: "concept:xyz", fields: {
        source: "{\"description\":\"A person\",\"with\":{\"name\":{\"the\":\"x/name\",\"as\":\"Text\"}}}" } }] }],
    }),
    rule: result({
      matches_before: [{ label: "rule", results: [{ this: "rule:r", fields: { definition: JSON.stringify({
        polarity: "retract", rule: { "assert!": "person", when: [{ assert: "person", where: { this: { "?": { name: "this" } } } }] } }) } }] }],
      matches_after: [{ label: "rule", results: [{ this: "rule:r", fields: { definition: JSON.stringify({
        polarity: "retract", rule: { "assert!": "person", when: [{ assert: "person", where: { this: { "?": { name: "this" } } } }] } }) } }] }],
    }),
    keyword: result({
      matches_before: [{ label: "thing", results: [{ this: "id:t", fields: { role: "keyword:" } }] }],
      matches_after: [{ label: "thing", results: [{ this: "id:t", fields: { role: "keyword:" } }] }],
    }),
    asserts: [
      el.asserts("person!:\n  name: x"),
      el.asserts("person!: &alice\n  name: x"),
      el.asserts("person:\n  this: ?x"),
      el.asserts("# person!:\nperson:\n  this: ?x"),
      el.asserts("  person!:"),
    ],
  };
});
check("a failure renders an escaped danger callout",
  rendered.failure.includes('wa-callout variant="danger"') && rendered.failure.includes("boom &lt;bad&gt;"), rendered.failure);
check("an empty result is the revision badge, its hash shortened",
  rendered.empty.includes("wa-badge") && rendered.empty.includes(">abcdefgh<"), rendered.empty);
check("a byte-array tree reads as #base58", rendered.bytes.includes('title="#1kA3B2yGe2z4"'), rendered.bytes);
check("an unchanged result lists notation in the switchable views",
  rendered.generic.includes("person!:") && rendered.generic.includes("tonk-cm-entity") && rendered.generic.includes("Alice")
    && rendered.generic.includes("evaluate-tabs") && rendered.generic.includes("query-tree") && rendered.generic.includes("query-table"),
  rendered.generic);
check("a commit that changed the result compares before and after",
  rendered.changed.includes("wa-comparison") && rendered.changed.includes("evaluate-side-before") && rendered.changed.includes("evaluate-side-after"),
  rendered.changed);
check("a concept draws its descriptor with notation types",
  rendered.concept.includes("concept!:") && rendered.concept.includes("description") && rendered.concept.includes(">text<"), rendered.concept);
check("a retract rule draws as rule!: with retract! and ?variables",
  rendered.rule.includes("rule!:") && rendered.rule.includes(">retract!<") && rendered.rule.includes(">?this<") && !rendered.rule.includes("assert!<"),
  rendered.rule);
check("a bare-scheme value is spelled as a :keyword", rendered.keyword.includes(">:keyword<"), rendered.keyword);
check("only a top-level head ending in ! asserts", JSON.stringify(rendered.asserts) === JSON.stringify([true, true, false, false, false]),
  JSON.stringify(rendered.asserts));

const diagnostics = await inspector.evaluate(() => {
  const el = document.createElement("space-inspector");
  const info = {
    name: "did:key:zSpaceRoute", label: "Research notes", subject: "did:key:zSpaceSubject", operator: "did:key:zOperator",
    profile: "did:key:zProfile",
    branch: { main: { revision: { tree: [0, 1, 2, 3] }, upstream: { remote: "origin", branch: "main" } } },
    remote: { origin: { address: { Ucan: { endpoint: "https://access.example.test/ucan/" } }, subject: "did:key:zRemoteSubject" } },
  };
  const local = { name: "local-space", label: "Offline draft", subject: "did:key:zLocal", operator: "did:key:zOperator",
    profile: "did:key:zProfile", branch: { main: { revision: null, upstream: null } } };
  const head = { ...local, branch: { main: { revision: { tree: "#local-head" }, upstream: null } } };
  const probed = { name: "space", label: "Space", subject: "did:key:zSpace", operator: "did:key:zOperator", profile: "did:key:zProfile",
    branch: { main: { revision: { tree: "#local-head" }, upstream: { remote: "origin", branch: "main" } } },
    remote: { origin: { address: { Memory: {} } } } };
  const broken = el.panelHtml("repository", "x", "main", "{", { state: "idle" });
  return {
    full: el.panelHtml("repository", "did:key:zSpaceRoute", "main", JSON.stringify(info), { state: "idle" }),
    local: el.panelHtml("repository", "local-space", "main", JSON.stringify(local), { state: "idle" }),
    head: el.panelHtml("repository", "local-space", "main", JSON.stringify(head), { state: "idle" }),
    probed: el.panelHtml("repository", "space", "main", JSON.stringify(probed),
      { state: "response", body: JSON.stringify({ state: "behind", local: { tree: "#local-head" }, remote: { tree: "#remote-head" } }) }),
    failure: el.panelHtml("failure", "space", "main", "offline <script>alert(1)</script>"),
    broken,
  };
});
check("full diagnostics name the space, its remote and its head",
  ["Research notes", "did:key:zSpaceRoute", "did:key:zSpaceSubject", "did:key:zProfile", "did:key:zOperator", "origin/main",
    "https://access.example.test/ucan/", "did:key:zRemoteSubject", "#1Ldp", "probe remote", 'data-copy-value="#1Ldp"']
    .every((expected) => diagnostics.full.includes(expected)), diagnostics.full);
check("a space with no upstream is local only",
  diagnostics.local.includes("none — local only") && diagnostics.local.includes("not configured")
    && diagnostics.local.includes("no commits") && !diagnostics.local.includes("probe remote"), diagnostics.local);
check("diagnostics stay collapsed until expanded",
  diagnostics.head.startsWith('<details class="inspector-debug__disclosure">')
    && ["branch diagnostics", "main", "#local-head", "local only", "copy revision", 'aria-live="polite"'].every((s) => diagnostics.head.includes(s)),
  diagnostics.head);
check("a probe shows the remote beside the local metadata",
  ["behind", "#local-head", "#remote-head", "did:key:zProfile"].every((s) => diagnostics.probed.includes(s)), diagnostics.probed);
check("a metadata failure is escaped and offers refresh",
  diagnostics.failure.includes("offline &lt;script&gt;alert(1)&lt;/script&gt;") && !diagnostics.failure.includes("<script>")
    && diagnostics.failure.includes('data-debug-action="refresh"'), diagnostics.failure);
check("undecodable metadata renders as the failure",
  diagnostics.broken.includes("metadata unavailable") && diagnostics.broken.includes("repository metadata decode"), diagnostics.broken);

const mounted = await inspector.evaluate(async () => {
  const space = document.createElement("space-inspector");
  space.setAttribute("with", "main@did:key:zSpace");
  const profile = document.createElement("space-inspector");
  profile.setAttribute("with", "main@profile:tonk");
  const bare = document.createElement("space-inspector");
  document.body.append(space, profile, bare);
  for (let i = 0; i < 40 && space.querySelector(".inspector-requires")?.hidden !== false; i++) await new Promise((r) => setTimeout(r, 25));
  for (let i = 0; i < 40 && !space.querySelector(".inspector-debug__rows"); i++) await new Promise((r) => setTimeout(r, 25));
  return {
    spacePanel: !!space.querySelector(".inspector-debug"),
    profilePanel: !!profile.querySelector(".inspector-debug"),
    rows: !!space.querySelector(".inspector-debug__rows"),
    notice: space.querySelector(".inspector-requires")?.hidden === false,
    sources: [space, profile].map((el) => el.querySelector("code-editor")?.getAttribute("source")),
    bare: bare.textContent,
    watching: globalThis.__watchers.size,
  };
});
check("a space's inspector shows its branch diagnostics, the profile's does not",
  mounted.spacePanel && !mounted.profilePanel && mounted.rows, JSON.stringify(mounted));
check("each cell's buffer names its branch",
  mounted.sources[0] === "tonk-buffer:///did:key:zSpace/main/scratch-0" && mounted.sources[1] === "tonk-buffer:///profile:tonk/main/scratch-0",
  JSON.stringify(mounted.sources));
check("without code-editor it says to install it", mounted.notice && mounted.watching > 0, JSON.stringify(mounted));
check("without a with it says so", mounted.bare.startsWith("no repository in context"), mounted.bare);

// Installing code-editor: the space now names it, the definition lands, and
// the waiting cells become editors.
await inspector.evaluate(() => {
  globalThis.__installed = true;
  for (const watcher of globalThis.__watchers) watcher.update?.({}, { tag: "code-editor" });
});
await define(inspector, "code-editor.yaml", "code-editor");
const installed = await inspector.evaluate(async () => {
  const space = document.querySelector('space-inspector[with="main@did:key:zSpace"]');
  for (let i = 0; i < 40 && !space.querySelector("code-editor")?.view; i++) await new Promise((r) => setTimeout(r, 25));
  return { notice: space.querySelector(".inspector-requires")?.hidden, editor: !!space.querySelector("code-editor")?.view };
});
check("once code-editor is installed the notice goes and the cell is an editor", installed.notice === true && installed.editor,
  JSON.stringify(installed));

// A clean query previews as a dry run; an assertion offers the bolt; a
// submit commits, seals and opens the next cell.
const type = (text) => inspector.evaluate(async (text) => {
  const space = document.querySelector('space-inspector[with="main@did:key:zSpace"]');
  const editor = [...space.querySelectorAll("code-editor")].at(-1);
  const before = globalThis.__evaluations.length;
  // The language server's frame for this text is the cue; the first one
  // waits out the service worker this page never gets.
  const frame = new Promise((resolve) => {
    const heard = (event) => {
      if (event.detail.value !== text) return;
      editor.removeEventListener("diagnostics", heard);
      resolve();
    };
    editor.addEventListener("diagnostics", heard);
  });
  editor.view.dispatch({ changes: { from: 0, to: editor.view.state.doc.length, insert: text }, userEvent: "input" });
  await Promise.race([frame, new Promise((r) => setTimeout(r, 5_000))]);
  await new Promise((r) => setTimeout(r, 50));
  const form = editor.closest("form");
  return {
    evaluations: globalThis.__evaluations.slice(before),
    play: form.querySelector(".evaluate-play").classList.contains("is-visible"),
    result: form.querySelector(".evaluate-slot").innerHTML,
    errors: editor.errorCount,
  };
}, text);
const preview = await type("person:\n  this: ?x");
check("a clean query previews as a dry run on the inspector's branch",
  preview.evaluations.length === 1 && preview.evaluations[0].transact === false && preview.evaluations[0].with === "main@did:key:zSpace"
    && preview.result.includes("evaluate-revision") && !preview.play, JSON.stringify(preview));
const broken = await type("bad: 1");
check("a buffer with errors does not run", broken.evaluations.length === 0 && broken.errors === 1 && !broken.play, JSON.stringify(broken));
const assertion = await type("person!:\n  name: \"Alice\"");
check("a clean assertion previews and offers the bolt",
  assertion.evaluations.length === 1 && assertion.evaluations[0].transact === false && assertion.play, JSON.stringify(assertion));

const committed = await inspector.evaluate(async () => {
  const space = document.querySelector('space-inspector[with="main@did:key:zSpace"]');
  const panelReads = globalThis.__fetched.filter((url) => url === "/api/repository/did:key:zSpace").length;
  let heard = 0;
  window.addEventListener("tonk:committed", () => { heard += 1; });
  const before = globalThis.__evaluations.length;
  space.querySelector(".evaluate-play").click();
  for (let i = 0; i < 40 && space.querySelectorAll("form").length < 2; i++) await new Promise((r) => setTimeout(r, 25));
  await new Promise((r) => setTimeout(r, 50));
  const forms = [...space.querySelectorAll("form")];
  return {
    commit: globalThis.__evaluations.slice(before),
    sealed: forms[0].classList.contains("cell-sealed"),
    readonly: forms[0].querySelector("code-editor").hasAttribute("readonly"),
    bolt: forms[0].querySelector(".evaluate-play").classList.contains("is-visible"),
    next: forms[1]?.querySelector("code-editor")?.getAttribute("source"),
    heard,
    refreshed: globalThis.__fetched.filter((url) => url === "/api/repository/did:key:zSpace").length > panelReads,
  };
});
check("a submit commits, seals the cell and opens the next",
  committed.commit.length === 1 && committed.commit[0].transact === true && committed.sealed && committed.readonly && !committed.bolt
    && committed.next === "tonk-buffer:///did:key:zSpace/main/scratch-1",
  JSON.stringify(committed));
check("a commit is announced and refreshes the diagnostics", committed.heard === 1 && committed.refreshed, JSON.stringify(committed));

await inspector.evaluate(() => {
  globalThis.__answer = (notation, transact) => {
    if (transact) throw { kind: "Rejected", message: "nope <here>" };
    return { matches_before: [], matches_after: [] };
  };
});
await type("person!:\n  name: \"Bob\"");
const refused = await inspector.evaluate(async () => {
  const space = document.querySelector('space-inspector[with="main@did:key:zSpace"]');
  const form = [...space.querySelectorAll("form")].at(-1);
  form.querySelector("code-editor").dispatchEvent(new CustomEvent("run", { bubbles: true }));
  for (let i = 0; i < 40 && !form.querySelector("wa-callout"); i++) await new Promise((r) => setTimeout(r, 25));
  return {
    callout: form.querySelector('wa-callout[variant="danger"]')?.textContent,
    sealed: form.classList.contains("cell-sealed"),
    forms: space.querySelectorAll("form").length,
  };
});
check("a refused commit stays on its cell with the reason", refused.callout === "nope <here>" && !refused.sealed && refused.forms === 2,
  JSON.stringify(refused));

const probe = await inspector.evaluate(async () => {
  const space = document.querySelector('space-inspector[with="main@did:key:zSpace"]');
  space.querySelector("[data-debug-action=probe]").click();
  for (let i = 0; i < 40 && !space.querySelector(".inspector-debug__rows")?.textContent.includes("behind"); i++) {
    await new Promise((r) => setTimeout(r, 25));
  }
  return { text: space.querySelector(".inspector-debug__rows")?.textContent, asked: globalThis.__fetched.includes("/api/repository/did:key:zSpace/branch/main/sync/status") };
});
check("probing the remote reads its sync state on request", probe.asked && probe.text.includes("behind") && probe.text.includes("#remote-head"),
  JSON.stringify(probe));

check("no errors on the pages", problems.length === 0, problems.join("\n"));
await browser.close();

for (const { name, ok, detail } of results) {
  console.log(`${ok ? "ok  " : "FAIL"} ${name}${detail ? `\n       ${detail}` : ""}`);
}
const failed = results.filter((r) => !r.ok).length;
console.log(`\n${results.length - failed} passed, ${failed} failed`);
process.exit(failed ? 1 : 0);
