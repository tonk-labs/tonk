// The library's notebook (notebook.yaml): <notebook-editor> and
// <notebook-index>, driven in a real Chromium the way a space renders them.
// Each definition is read out of its YAML (every `!include/text` inlined)
// and handed to the real element runtime, beside the library's own
// <prose-editor> and <code-editor>; the worker is a stand-in behind `fetch`
// answering `/evaluate`, `/query` and a language server.
//
// First the notebook's pure halves (`blocks()`, `positions()`,
// `notation()`, `result()`), against the cases the Rust notebook was tested
// on and against positions dialog derived itself (`fixtures/`). Then the
// element: projecting rows, committing edits block by block, cells, the
// index's switcher, and what it says while an editor is missing.
//
// Run it (after `npm run build` here):
//
//   node rust/tonk-code/tests/notebook-elements.mjs
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
const FIXTURES = JSON.parse(readFileSync(join(here, "fixtures/notebook-positions.json"), "utf8"));

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

/** The language records code-editor.yaml seeds: `this`, words, pack. */
function records(file) {
  const out = [];
  const text = readFileSync(join(library, file), "utf8");
  for (const block of text.split(/\n(?=\S)/)) {
    const id = block.match(/^code-editor\/language(?:\/word)?!:\n  this: (\S+)/)?.[1];
    if (!id) continue;
    let record = out.find((r) => r.this === id);
    if (!record) out.push((record = { this: id, words: [], source: null }));
    const word = block.match(/\n  word: (".*")/)?.[1];
    if (word) record.words.push(JSON.parse(word));
    const source = block.match(/\n  source: !include\/text (\S+)/)?.[1];
    if (source) record.source = readFileSync(join(library, source), "utf8");
  }
  return out;
}

const results = [];
const check = (name, ok, detail = "") => results.push({ name, ok, detail: ok ? "" : detail });

const browser = await chromium.launch({ args: ["--no-proxy-server"] });
const problems = [];

/** A page with the runtime and a stand-in worker: evaluations answer from
 *  `__answer`, a name check from `__installed`, the language server finds
 *  an error wherever a document says `bad`. */
const open = async () => {
  const page = await browser.newPage();
  page.on("console", (m) => { if (m.type() === "error") problems.push(m.text()); });
  page.on("pageerror", (e) => problems.push(String(e)));
  await page.setContent("<!doctype html><html><head></head><body></body></html>");
  await page.evaluate((languages) => {
    document.addEventListener("tonk-query", (event) => {
      event.preventDefault();
      const word = event.detail.query?.terms?.word;
      event.detail.result = Promise.resolve(languages
        .filter((r) => r.words.includes(word) && r.source)
        .map((r) => ({ this: r.this, fields: { this: r.this, word, source: r.source } })));
    });
    document.addEventListener("tonk-subscribe", (event) => {
      event.preventDefault();
      event.detail.subscription = { cancel: () => {} };
    });
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
    globalThis.__evaluations = [];
    globalThis.__queries = [];
    globalThis.__installed = new Set(["prose-editor", "code-editor"]);
    globalThis.__answer = () => ({ matches_before: [], matches_after: [] });
    window.fetch = async (url, init = {}) => {
      const evaluate = url.match(/^\/api\/repository\/([^/]+)\/branch\/([^/]+)\/evaluate(\?transact=false)?$/);
      if (evaluate) {
        const transact = !evaluate[3];
        globalThis.__evaluations.push({ repo: evaluate[1], branch: evaluate[2], transact, notation: init.body });
        try {
          return new Response(JSON.stringify(await globalThis.__answer(init.body, transact)));
        } catch (error) {
          return new Response(JSON.stringify({ error: { message: error.message } }), { status: 400 });
        }
      }
      const query = url.match(/^\/api\/repository\/([^/]+)\/branch\/([^/]+)\/query$/);
      if (query) {
        const body = JSON.parse(init.body);
        globalThis.__queries.push({ repo: query[1], body });
        const name = String(body.terms?.this ?? "").replace(/^id:/, "");
        return new Response(JSON.stringify(globalThis.__installed.has(name) ? [{ this: body.terms.this, fields: {} }] : []));
      }
      if (url !== "/api/language-server") return new Response("", { status: 404 });
      if (init.method !== "POST") {
        return new Response(new ReadableStream({ start: (controller) => streams.add(controller) }), {
          headers: { "content-type": "text/event-stream" },
        });
      }
      const message = JSON.parse(init.body);
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
    globalThis.__navigated = [];
    window.tonk = { navigate: (href) => globalThis.__navigated.push(href) };
    globalThis.__wait = async (predicate, ms = 10_000) => {
      const deadline = Date.now() + ms;
      for (;;) {
        const value = predicate();
        if (value) return value;
        if (Date.now() > deadline) return value;
        await new Promise((r) => setTimeout(r, 25));
      }
    };
  }, records("code-editor.yaml"));
  await page.addScriptTag({ content: RUNTIME });
  return page;
};
const define = async (page, file, tag) => {
  await page.addScriptTag({ type: "module", content: module(tag, definition(file, tag)) });
  await page.waitForFunction((tag) => customElements.get(tag), tag);
};

// ---------------------------------------------------------------------------
// The pure halves
// ---------------------------------------------------------------------------

const page = await open();
await define(page, "notebook.yaml", "notebook-editor");

const pure = await page.evaluate((fixtures) => {
  const el = document.createElement("notebook-editor");
  const B = el.blocks();
  const P = el.positions();
  const N = el.notation();
  const out = {};
  const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);
  const stored = (pairs) => pairs.map(([entity, source, key = null]) => ({ entity, source, key }));

  // Splitting and projecting.
  const doc = "intro\n\n# Heading\n\nunder it\n\ntrailing";
  out.spans = eq(B.split(doc), ["intro", "# Heading\n\nunder it", "trailing"]) && eq(B.blockSpans(doc), [[0, 1], [1, 2], [3, 1]]);
  out.headingRun = B.split("# Title\n\n## Subtitle\n\nbody").length === 1 && eq(B.blockSpans("# Title\n\n## Subtitle\n\nbody"), [[0, 3]]);
  out.spanAt = eq([0, 1, 2, 3, 9].map((n) => B.spanAt(doc, n)), [[0, 1], [1, 2], [1, 2], [3, 1], null]);
  out.spanEvery = ["one", "one\n\ntwo", "# a\n\nb\n\n# c\n\nd", "```dialog\nconcept:\n```\n\nafter", "# only a heading"]
    .every((d) => B.blockSpans(d).length === B.split(d).length);
  out.paragraphs = eq(B.split("first\n\nsecond\n\nthird"), ["first", "second", "third"]);
  out.fence = eq(B.split("intro\n\n```dialog\nperson:\n\n  name: ?name\n```\n\nafter"),
    ["intro", "```dialog\nperson:\n\n  name: ?name\n```", "after"]);
  out.adjacent = eq(B.split("- one\n- two\n- three\n\nafter"), ["- one\n- two\n- three", "after"]);
  out.collapse = eq(B.split("first\n\n\n\nsecond"), ["first", "second"]);
  out.delimiter = eq(B.split("~~~\n```\n\nstill inside\n~~~\n\nafter"), ["~~~\n```\n\nstill inside\n~~~", "after"]);
  out.closerLength = eq(B.split("````\n```\n\ninside\n````\n\nafter"), ["````\n```\n\ninside\n````", "after"]);
  out.unclosed = eq(B.split("intro\n\n```dialog\nperson:\n\nstill typing"), ["intro", "```dialog\nperson:\n\nstill typing"]);
  out.project = B.project(["first", "second"]) === "first\n\nsecond";
  out.lineBreak = B.split("first line\nsecond line").length === 1 && B.project(B.split("first line\nsecond line")) === "first line\nsecond line";
  const trailing = "```dialog-yaml\ncounter/model!:\n  this: id:counter/1\n\n\n```";
  out.trailingBlank = B.project(B.split(trailing)) === trailing;
  const blocks = ["# Title\n\nA paragraph.", "```dialog\nperson:\n\n  name: ?name\n```", "- a\n- b"];
  out.roundTrip = eq(B.split(B.project(blocks)), blocks);
  out.group = eq(B.split("# Title\n\nA paragraph.\n\n# Next\n\nMore."), ["# Title\n\nA paragraph.", "# Next\n\nMore."]);
  out.groupRun = eq(B.split("# Title\n\n## Subtitle\n\nA paragraph."), ["# Title\n\n## Subtitle\n\nA paragraph."]);
  out.groupFence = eq(B.split("## Query\n\n```dialog\nconcept:\n```\n\nAfter."), ["## Query\n\n```dialog\nconcept:\n```", "After."]);
  out.firstChunk = eq(B.split("# Title\n\nFirst.\n\nSecond."), ["# Title\n\nFirst.", "Second."]);
  out.trailingHeading = eq(B.split("A paragraph.\n\n# Just typed"), ["A paragraph.", "# Just typed"]);
  out.bareHash = eq(B.split("#hashtag\n\nA paragraph."), ["#hashtag", "A paragraph."]);
  const seeded = ["# Scratch\n\nA `dialog-yaml` fence is a live cell.", "```dialog-yaml\nconcept:\n```", "Query cells run on their own."];
  const ungrouped = ["# Scratch", "A `dialog-yaml` fence is a live cell.", "Query cells run on their own."];
  out.seed = eq(B.split(B.project(seeded)), seeded) && !eq(B.split(B.project(ungrouped)), ungrouped);
  out.empty = B.project([]) === "" && eq(B.split(""), []);

  // Titles.
  out.titles = [
    B.titleOf("# Notes\n\nbody"), B.titleOf("### Deep\n\nbody"), B.titleOf("# Notes #\n"),
    B.titleOf("just a paragraph\n\n# Later"), B.titleOf("#\n"), B.titleOf("####### seven\n"),
  ];

  // Reconciling an edit with the stored blocks.
  const abc = stored([["a", "first"], ["b", "second"], ["c", "third"]]);
  const moved = B.reconcile(abc, ["third", "first", "second"]);
  out.reorder = !moved.changed.length && !moved.created.length && !moved.removed.length && moved.reordered && eq(moved.order, ["c", "a", "b"]);
  const edited = B.reconcile(abc, ["first", "second edited", "third"]);
  out.edited = eq(edited.changed, [["b", "second edited"]]) && !edited.created.length && !edited.removed.length && !edited.reordered;
  const inserted = B.reconcile(stored([["a", "first"], ["b", "second"]]), ["first", "middle", "second"]);
  out.inserted = eq(inserted.created, ["middle"]) && !inserted.changed.length && eq(inserted.order, ["a", null, "b"]) && inserted.reordered;
  const removed = B.reconcile(abc, ["first", "third"]);
  out.removed = eq(removed.removed, ["b"]) && !removed.changed.length && eq(removed.order, ["a", "c"]);
  const duplicates = B.reconcile(stored([["a", "same"], ["b", "same"]]), ["same", "same"]);
  out.duplicates = !duplicates.changed.length && !duplicates.removed.length && !duplicates.created.length && !duplicates.reordered;
  const both = B.reconcile(abc, ["third", "first edited", "second"]);
  out.moveAndEdit = eq(both.changed, [["a", "first edited"]]) && eq(both.order, ["c", "a", "b"]);
  const distant = B.reconcile(abc, ["new", "first", "second"]);
  out.distant = !distant.changed.length && eq(distant.created, ["new"]) && eq(distant.removed, ["c"]);
  const beside = B.reconcile(stored([["a", "first"], ["b", "second"]]), ["first", "inserted", "second, edited"]);
  out.beside = eq(beside.changed, [["b", "second, edited"]]) && eq(beside.created, ["inserted"]) && !beside.removed.length && eq(beside.order, ["a", null, "b"]);
  const split = B.reconcile(stored([["a", "hello there, world"]]), ["hello there,", "world"]);
  out.split = eq(split.changed, [["a", "hello there,"]]) && eq(split.created, ["world"]) && !split.removed.length;
  out.unchanged = eq(B.reconcile(stored([["a", "first"], ["b", "second"]]), ["first", "second"]),
    { changed: [], created: [], removed: [], order: ["a", "b"], reordered: false });

  // Keys carried across a missing entry; inserts landing; respelling.
  const base = stored([["id:a", "A", "N1"], ["id:b", "B", "N5"], ["id:c", "C", "N9"]]);
  const rows = stored([["id:a", "A", "N1"], ["id:c", "C", "N9"], ["id:b", "B"]]);
  B.sortByKey(B.carryKeys(rows, base));
  out.carry = eq(rows.map((b) => b.entity), ["id:a", "id:b", "id:c"]) && rows[1].key === "N5"
    && P.assignKeys(rows.map((b) => [b.entity, b.key])).length === 0;
  const never = stored([["id:a", "A", "N1"], ["id:b", "B"]]);
  B.carryKeys(never, stored([["id:a", "A", "N1"], ["id:b", "B"]]));
  out.never = never[1].key === null;
  const rendered = stored([["id:a", "A", "N7"]]);
  B.carryKeys(rendered, stored([["id:a", "A", "N1"]]));
  out.rendered = rendered[0].key === "N7";
  const landedBase = stored([["id:a", "A", "N1"]]);
  const keyless = stored([["id:a", "A", "N1"], ["id:new", "Hello world"]]);
  const keyed = stored([["id:a", "A", "N1"], ["id:new", "Hello world", "N3"]]);
  out.landed = B.landedInserts(keyless, landedBase, ["Hello world"], false).length === 0
    && B.landedInserts(keyless, landedBase, ["Hello world"], true).length === 1
    && B.landedInserts(keyed, landedBase, ["Hello world"], false)[0]?.entity === "id:new";
  const respelled = stored([["a", "ends with a space "], ["b", "remote edit"]]);
  B.respell(respelled, new Map([["a", "ends with a space "], ["b", "before"]]), stored([["a", "ends with a space"], ["b", "before"]]));
  const first = stored([["a", "ends with a space "], ["b", "remote edit"]]);
  B.respell(first, new Map(), stored([["a", "ends with a space"], ["b", "before"]]));
  out.respell = respelled[0].source === "ends with a space" && respelled[1].source === "remote edit"
    && eq(first, stored([["a", "ends with a space "], ["b", "remote edit"]]));

  // Keys between kept neighbours.
  const a = P.assignKeys([["id:a", null]]);
  const ka = a[0][1];
  const appended = P.assignKeys([["id:a", ka], ["id:b", null]]);
  const kb = appended[0][1];
  const between = P.assignKeys([["id:a", ka], ["id:c", null], ["id:b", kb]]);
  const kc = between[0][1];
  const shuffled = P.assignKeys([["id:b", kb], ["id:a", ka], ["id:c", kc]]);
  out.keys = a.length === 1 && appended.length === 1 && ka < kb && between.length === 1 && ka < kc && kc < kb
    && shuffled.length === 1 && shuffled[0][0] === "id:b" && shuffled[0][1] < ka
    && P.assignKeys([["id:a", ka], ["id:b", kb]]).length === 0
    && ["N1", "N5", "N9"].every(P.parse);

  // Positions as dialog derives them, BLAKE3 underneath.
  const hex = (bytes) => bytes.map((b) => b.toString(16).padStart(2, "0")).join("");
  out.blake3 = hex(P.blake3([])) === "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
    && hex(P.blake3([...new TextEncoder().encode("abc")])) === "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85";
  const alphabet = (n) => Array.from({ length: n }, (_, i) => String.fromCharCode(97 + (i % 26))).join("");
  const member = (m) => (typeof m === "string" ? m : m.repeat ? m.repeat.repeat(m.times) : alphabet(m.alphabet));
  out.fixtureMisses = fixtures.inserts.filter(([m, low, high, want]) => P.insert(member(m), low, high) !== want)
    .slice(0, 5).map(([m, low, high, want]) => ({ member: member(m).slice(0, 40), low, high, want, got: P.insert(member(m), low, high) }));
  out.fixtureCount = fixtures.inserts.length;
  out.assignMisses = fixtures.assign.filter(({ order, keys }) => !eq(P.assignKeys(order), keys));
  out.canonical = ["N0", "O000", "N0z0", "", "0N"].every((bad) => !P.parse(bad)) && ["N", "O1z", "A1"].every(P.parse);

  // Notation.
  const insertOne = N.inserts("id:nb", ["id:a", null, "id:b"], ["hello"]);
  out.insertOne = insertOne.includes("this: ?b0") && insertOne.includes("next: id:b") && insertOne.includes("prev: id:a");
  const run = N.inserts("id:nb", ["id:a", null, null, "id:b"], ["one", "two"]);
  out.chain = run.includes("this: ?b0") && run.includes("this: ?b1") && run.includes("next: ?b1") && run.includes("next: id:b")
    && run.indexOf("this: ?b1") < run.indexOf("next: ?b1");
  const tail = N.inserts("id:nb", ["id:a", null], ["last"]);
  out.tail = tail.includes("next: case:none") && tail.includes("prev: id:a");
  const head = N.inserts("id:nb", [null, "id:b"], ["first"]);
  out.head = head.includes("prev: tonk:notebook/edge") && head.includes("next: id:b");
  out.nothing = N.inserts("id:nb", ["id:a"], []) === null;
  // Markdown survives as a double-quoted scalar: JSON string syntax, which
  // YAML reads as the same text.
  const hostile = ["```dialog\nnotebook:\n  title: ?t\n```", "    indented code\nnot indented", "  - nested item\n- top item",
    "?looks-like-a-variable", "trailing spaces   ", "```dialog-yaml\nconcept:\n\n\n```", "tab\tand \"quotes\" and \\ backslash",
    "# Heading: with a colon"];
  out.verbatim = hostile.every((source) => {
    const text = N.inserts("id:nb", [null], [source]);
    const line = text.split("\n").find((row) => row.startsWith("  source: "));
    return JSON.parse(line.slice("  source: ".length)) === source;
  });
  const keyA = P.assignKeys([["id:a", null]])[0][1];
  const keyB = P.assignKeys([["id:a", keyA], ["id:b", null]])[0][1];
  const keyC = P.assignKeys([["id:b", keyB], ["id:c", null]])[0][1];
  const four = stored([["id:a", "first", keyA], ["id:b", "second", keyB], ["id:c", "third", keyC], ["id:d", "fourth"]]);
  const whole = N.commit("id:nb", four, B.reconcile(four, ["third", "first", "second edited", "new"]), "Renamed");
  out.commit = whole.includes("block/edit!:\n  subject: id:b\n") && whole.includes("block/remove!:\n  subject: id:d\n")
    && whole.includes('key: ""') && whole.includes(`block/move!:\n  subject: id:c\n  notebook: id:nb\n  from: ${JSON.stringify(keyC)}\n`)
    && (whole.match(/block\/move!:/g) ?? []).length + (whole.match(/block\/place!:/g) ?? []).length === 1
    && whole.includes("block/insert!:") && whole.includes("prev: id:b") && whole.includes("notebook/retitle!:")
    && ["second edited", "new", "Renamed"].every((value) => whole.includes(JSON.stringify(value)));
  out.commitText = whole;
  const one = stored([["id:a", "first", keyA]]);
  out.noCommit = N.commit("id:nb", one, B.reconcile(one, ["first"]), null) === null;
  out.draftBlocks = eq(N.draftBlocks("# Counter\n\nfirst\nstill first\n\nsecond"), ["# Counter", "first\nstill first", "second"])
    && N.draftBlocks("\n\n  \n").length === 0;
  out.quotedTitle = N.create("notebook:n1", 'a: [b] "c"', "").includes('  title: "a: [b] \\"c\\""');
  const created = N.create("notebook:n1", "T", "one\n\ntwo");
  out.created = created.indexOf("this: ?b1") < created.indexOf("this: ?b0") && created.includes("next: ?b1")
    && created.includes("next: case:none") && (N.create("notebook:n1", "T", "").match(/block\/insert!:/g) ?? []).length === 1;
  out.scalar = N.blockScalar("a: b\n`c`") === "|-\n    a: b\n    `c`";

  // A cell's output.
  const result = (failure, response) => el.result(failure, response, "main@id:repo");
  const card = (row, label) => result(null, { matches_after: [{ label, results: [row] }] });
  const row = (thing, fields, transient = false) => ({ this: thing, fields, transient });
  out.transient = (() => {
    const html = card(row("did:key:z6MkCommand", { subject: "did:key:z6MkCounter" }, true), "counter/+1");
    return !html.includes("<tonk-display") && html.includes("subject") && html.includes("did:key:z6MkCounter");
  })();
  out.persisted = card(row("did:key:z6MkCounter", { count: "+0" }), "counter/model").includes("<tonk-display");
  out.nothingRendered = result(null, null) === "";
  const emptyBlock = result(null, { matches_after: [{ label: "person", results: [] }] });
  out.emptyBlock = emptyBlock.includes("no results") && !emptyBlock.includes("nb-out__gallery");
  const forty = result(null, { matches_after: [{ label: "thing", results: Array.from({ length: 40 }, (_, n) => row(`id:thing/${n}`, { name: "x" })) }] });
  const [shown, folded] = forty.split("<details");
  out.fold = (shown.match(/class="nb-card /g) ?? []).length === 12 && (folded.match(/class="nb-card /g) ?? []).length === 28
    && folded.includes("28 more</summary>") && forty.includes("40 results");
  out.short = !result(null, { matches_after: [{ label: "thing", results: [row("id:thing/1", {})] }] }).includes("<details");
  const modelled = card(row("id:notebook/scratch", { model: "tonk:notebook" }), "concept");
  out.modelled = modelled.includes("<tonk-display") && modelled.includes('entity="id:notebook/scratch"') && modelled.includes('model="tonk:notebook"');
  const concept = card(row("db:person", {
    source: '{"description":"A person: someone.","with":{"name":{"the":"xyz.person/name","as":"Text"}}}', transient: false,
  }), "concept");
  out.concept = !concept.includes("<tonk-display") && concept.includes("<tonk-notation>") && concept.includes("concept!:\n  this: db:person\n")
    && concept.includes("  with:\n    name:\n") && concept.includes("\n      as: text") && concept.includes("\n      the: xyz.person/name")
    && concept.includes('description: "A person: someone."') && !concept.includes("{2}");
  const viewless = card(row("did:key:z6MkCommand", { block: { N1: "id:nb/1", N5: "id:nb/2" } }, true), "notebook/blocks");
  out.viewless = viewless.includes("notebook/blocks!:\n  this: did:key:z6MkCommand\n  block:\n    N1: id:nb/1\n    N5: id:nb/2");
  const person = card(row("id:alice", { name: "Alice" }), "person");
  out.byLabel = person.includes("<tonk-display") && person.includes('entity="id:alice"') && person.includes('model="person"');
  out.stamped = person.includes('with="main@id:repo"');
  out.namedModel = card(row("id:x", { model: "tonk:notebook" }), "person").includes('model="tonk:notebook"');
  out.headed = person.includes('nb-card__name">Alice<') && person.includes('nb-card__entity">id:alice<');
  const failure = result("boom <bad>", null);
  out.failure = failure.includes("nb-out--error") && failure.includes("boom &lt;bad&gt;");
  out.before = result(null, { matches_before: [{ label: "person", results: [] }], matches_after: [] }).includes("no results");

  out.asserts = [el.asserts("person!:\n  name: x"), el.asserts("person:\n  this: ?x"), el.asserts("# person!:\nperson:\n  this: ?x")];
  return out;
}, FIXTURES);

const pureChecks = {
  spans: "spans cover the nodes each block covers",
  headingRun: "a heading run spans its content",
  spanAt: "every node maps back to its block's span",
  spanEvery: "a span per block, whatever the document",
  paragraphs: "paragraphs split on blank lines",
  fence: "a fence stays whole despite inner blank lines",
  adjacent: "adjacent lines stay one block",
  collapse: "runs of blank lines collapse",
  delimiter: "a fence does not close on the other delimiter",
  closerLength: "a closer must be as long as its opener",
  unclosed: "an unclosed fence runs to the end",
  project: "blocks project with one blank line between",
  lineBreak: "a line break within a block survives",
  trailingBlank: "trailing blank lines inside a fence survive",
  roundTrip: "blocks round-trip through a document",
  group: "a heading groups with the content it introduces",
  groupRun: "consecutive headings group with one body",
  groupFence: "a heading groups with a following fence",
  firstChunk: "only the first chunk joins a heading",
  trailingHeading: "a trailing heading with no content is its own block",
  bareHash: "a bare hash is not a heading",
  seed: "the seeded blocks are already partitioned",
  empty: "an empty document round-trips",
  reorder: "a moved block reads as a reorder, not a rewrite",
  edited: "only the edited block is written",
  inserted: "an inserted block is created",
  removed: "a deleted block is removed",
  duplicates: "duplicate sources stay distinct",
  moveAndEdit: "a move and an edit separate in one pass",
  distant: "an insert never pairs with a distant deletion",
  beside: "an edited block keeps its identity beside an insert",
  split: "identity survives a split",
  unchanged: "nothing changed is no edit",
  carry: "a key carries across a missing entry",
  never: "a never-keyed block stays unplaced",
  rendered: "a rendered key wins over a carried one",
  landed: "an insert lands once its entry shows",
  respell: "an unchanged row keeps the editor's spelling",
  keys: "keys go between kept neighbours, and a moved block is the one re-keyed",
  blake3: "BLAKE3 digests match the reference vectors",
  canonical: "only canonical positions parse",
  insertOne: "one inserted block names its neighbours",
  chain: "a run of inserts chains by variable, bound before use",
  tail: "a run at the tail ends at case:none",
  head: "a run at the head follows the edge",
  nothing: "no inserts, no notation",
  verbatim: "markdown is carried verbatim",
  commit: "a whole commit is one document",
  noCommit: "an unchanged document commits nothing",
  draftBlocks: "a draft splits on blank lines and keeps its heading",
  quotedTitle: "a hostile title is quoted as data",
  created: "a create chains back to front and seeds an empty block",
  scalar: "a create carries sources as block scalars",
  transient: "a command's result is drawn from its fields",
  persisted: "a stored result is drawn by a display",
  nothingRendered: "no response renders nothing",
  emptyBlock: "an empty result is one summary line",
  fold: "results past the cap fold behind one line",
  short: "a short result does not fold",
  modelled: "a result naming a model goes to a display",
  concept: "a concept is shown as its definition",
  viewless: "a viewless result is its notation, nested as lines",
  byLabel: "a query's label is its result's model",
  stamped: "a card's display carries the notebook's context",
  namedModel: "a named model wins over the label",
  headed: "a drawn result is headed by its name and entity",
  failure: "a failure renders escaped, without a gallery",
  before: "a dry run's matches render when nothing committed",
};
for (const [key, name] of Object.entries(pureChecks)) check(name, pure[key] === true, JSON.stringify(pure[key]));
check("titles come from the leading heading only",
  JSON.stringify(pure.titles) === JSON.stringify(["Notes", "Deep", "Notes", null, null, null]), JSON.stringify(pure.titles));
check(`positions match dialog's own derivation (${pure.fixtureCount} cases)`, pure.fixtureMisses.length === 0, JSON.stringify(pure.fixtureMisses));
check("keys match the Rust notebook's assignment", pure.assignMisses.length === 0, JSON.stringify(pure.assignMisses));
check("only a top-level head ending in ! asserts", JSON.stringify(pure.asserts) === JSON.stringify([true, false, false]), JSON.stringify(pure.asserts));

// ---------------------------------------------------------------------------
// <notebook-editor>
// ---------------------------------------------------------------------------

await define(page, "code-editor.yaml", "code-editor");
await define(page, "prose-editor.yaml", "prose-editor");

/** Mount a notebook on `with`, its data pane holding `blocks`
 *  (`[entity, source, key, notebook]`), and wait for the projection. */
const mount = (blocks) => page.evaluate(async (blocks) => {
  const el = document.createElement("notebook-editor");
  el.setAttribute("with", "main@did:key:zSpace");
  el.dataset.subject = "id:nb";
  el.dataset.notebook = "id:nb";
  el.setAttribute("title", "Scratch");
  const pane = document.createElement("div");
  pane.className = "notebook-data";
  pane.hidden = true;
  const order = document.createElement("div");
  order.className = "notebook-order";
  const rows = document.createElement("div");
  rows.className = "notebook-block-rows";
  pane.append(order, rows);
  el.append(pane);
  globalThis.__row = (entity, source, key, notebook = "id:nb") => {
    let row = rows.querySelector(`[data-block="${entity}"]`);
    if (!row) {
      row = document.createElement("div");
      row.className = "notebook-block-row";
      row.dataset.block = entity;
      rows.append(row);
    }
    row.dataset.notebook = notebook;
    row.dataset.source = source;
    if (key) {
      let entry = order.querySelector(`[data-block="${entity}"]`);
      if (!entry) {
        entry = document.createElement("span");
        entry.className = "notebook-order__item";
        entry.dataset.block = entity;
      }
      entry.dataset.key = key;
      const later = [...order.children].find((item) => item.dataset.key > key && item !== entry);
      order.insertBefore(entry, later ?? null);
    }
  };
  globalThis.__unrow = (entity) => {
    rows.querySelector(`[data-block="${entity}"]`)?.remove();
    order.querySelector(`[data-block="${entity}"]`)?.remove();
  };
  for (const block of blocks) globalThis.__row(...block);
  document.body.append(el);
  globalThis.__notebook = el;
  await globalThis.__wait(() => el.querySelector("prose-editor")?.editor && el.__notebook?.projectedOnce);
  await new Promise((r) => setTimeout(r, 300));
  return el.querySelector("prose-editor").value;
}, blocks);

const FENCE = "```dialog-yaml\nperson:\n  this: ?x\n```";
await page.evaluate(() => {
  globalThis.__answer = () => ({ matches_after: [{ label: "person", results: [] }] });
});
const projected = await mount([
  ["id:b1", "# Scratch\n\nintro paragraph.", "N1"],
  ["id:b2", FENCE, "N5"],
  ["id:b3", "Last.", "N9"],
  ["id:other", "Not mine.", "N3", "id:else"],
  ["id:b4", "Unplaced."],
]);
check("the rows project in key order, only this notebook's, unplaced last",
  projected === `# Scratch\n\nintro paragraph.\n\n${FENCE}\n\nLast.\n\nUnplaced.`, JSON.stringify(projected));

/** What the notebook committed since `before` evaluations. */
const commits = (since) => page.evaluate((since) =>
  globalThis.__evaluations.slice(since).filter((e) => e.transact).map((e) => e.notation), since);
const count = () => page.evaluate(() => globalThis.__evaluations.length);

const cell = await page.evaluate(async () => {
  const root = globalThis.__notebook.querySelector("prose-editor").shadowRoot;
  const editor = await globalThis.__wait(() => root.querySelector(".md-code-block code-editor[source]"));
  const slot = await globalThis.__wait(() => {
    const found = root.querySelector(".md-code-block .notebook-cell-result");
    return found && found.innerHTML ? found : null;
  }, 8_000);
  return {
    source: editor?.getAttribute("source"),
    language: editor?.getAttribute("language"),
    runs: globalThis.__evaluations.filter((e) => !e.transact).map((e) => [e.repo, e.branch, e.notation]),
    result: slot?.innerHTML ?? null,
  };
});
check("a dialog fence is a cell on the branch's language server",
  /^tonk-buffer:\/\/\/did:key:zSpace\/main\/notebook-\d+$/.test(cell.source ?? "") && cell.language === "dialog-yaml", JSON.stringify(cell));
check("a clean query cell runs as a dry run on the notebook's branch, and shows its result",
  cell.runs.some(([repo, branch, notation]) => repo === "did:key:zSpace" && branch === "main" && notation === "person:\n  this: ?x")
    && cell.result?.includes("nb-out"), JSON.stringify(cell));
check("the first projection commits nothing", (await commits(0)).length === 0, JSON.stringify(await commits(0)));

/** Type `text` after `needle` in the notebook's document, as a keystroke. */
const typeAfter = (needle, text) => page.evaluate(({ needle, text }) => {
  const view = globalThis.__notebook.querySelector("prose-editor").editor.view;
  let at = null;
  view.state.doc.descendants((node, pos) => {
    if (at === null && node.isText && node.text.includes(needle)) at = pos + node.text.indexOf(needle) + needle.length;
  });
  view.dispatch(view.state.tr.insertText(text, at));
}, { needle, text });
/** Wait for a commit after `since`, and return the commits. */
const committed = async (since) => {
  await page.evaluate((since) => globalThis.__wait(() =>
    globalThis.__evaluations.slice(since).some((e) => e.transact), 5_000), since);
  await page.evaluate(() => new Promise((r) => setTimeout(r, 600)));
  return commits(since);
};

let mark = await count();
await typeAfter("Last.", " Edited");
let written = await committed(mark);
check("typing commits the one block it changed, and places the unplaced one",
  written.length >= 1 && written.join("").includes('block/edit!:\n  subject: id:b3\n  notebook: id:nb\n  source: "Last. Edited"')
    && (written.join("").match(/block\/edit!:/g) ?? []).length === 1
    && written.join("").includes("block/place!:\n  subject: id:b4\n  notebook: id:nb\n  key: ")
    && !written.join("").includes("notebook/retitle!:"), JSON.stringify(written));
// The worker's rows catch up.
await page.evaluate(() => {
  globalThis.__row("id:b3", "Last. Edited", "N9");
  const placed = globalThis.__evaluations.filter((e) => e.transact).map((e) => e.notation).join("")
    .match(/block\/place!:\n  subject: id:b4\n  notebook: id:nb\n  key: "([^"]+)"/)[1];
  globalThis.__row("id:b4", "Unplaced.", placed);
});
await page.evaluate(() => new Promise((r) => setTimeout(r, 400)));

mark = await count();
await page.evaluate(() => {
  const prose = globalThis.__notebook.querySelector("prose-editor");
  prose.value = `${prose.value}\n\nBrand new.`;
});
written = await committed(mark);
check("a new paragraph is inserted after the block it follows",
  written.join("").includes('block/insert!:\n  this: ?b0\n  notebook: id:nb\n  source: "Brand new."\n  next: case:none\n  prev: id:b4\n'),
  JSON.stringify(written));

// More typing while the insert is out waits for it to land.
mark = await count();
await typeAfter("Brand new.", " More");
await page.evaluate(() => new Promise((r) => setTimeout(r, 800)));
const waiting = await commits(mark);
check("nothing commits while an insert is on its way", waiting.length === 0, JSON.stringify(waiting));
await page.evaluate(() => globalThis.__row("id:new", "Brand new.", "NZ"));
written = await committed(mark);
check("once it lands, the typing commits onto the new block, not a second insert",
  written.join("").includes('block/edit!:\n  subject: id:new\n  notebook: id:nb\n  source: "Brand new. More"')
    && !written.join("").includes("block/insert!:"), JSON.stringify(written));
await page.evaluate(() => globalThis.__row("id:new", "Brand new. More", "NZ"));

mark = await count();
await typeAfter("Scratch", " Two");
written = await committed(mark);
check("editing the heading renames the notebook",
  written.join("").includes('notebook/retitle!:\n  subject: id:nb\n  title: "Scratch Two"')
    && written.join("").includes("block/edit!:\n  subject: id:b1\n"), JSON.stringify(written));
await page.evaluate(() => globalThis.__row("id:b1", "# Scratch Two\n\nintro paragraph.", "N1"));
await page.evaluate(() => new Promise((r) => setTimeout(r, 400)));

// Another replica edits a block.
mark = await count();
const remote = await page.evaluate(async () => {
  globalThis.__row("id:b3", "Last, from elsewhere.", "N9");
  const prose = globalThis.__notebook.querySelector("prose-editor");
  await globalThis.__wait(() => prose.value.includes("Last, from elsewhere."), 3_000);
  await new Promise((r) => setTimeout(r, 400));
  return prose.value;
});
check("another replica's edit is projected into the editor",
  remote.includes("Last, from elsewhere.") && !remote.includes("Last. Edited"), JSON.stringify(remote));
check("a remote edit is not written back", (await commits(mark)).length === 0, JSON.stringify(await commits(mark)));

mark = await count();
await page.evaluate(() => {
  const prose = globalThis.__notebook.querySelector("prose-editor");
  prose.value = prose.value.replace("\n\nLast, from elsewhere.", "");
});
written = await committed(mark);
check("deleting a block removes it by its key",
  written.join("").includes('block/remove!:\n  subject: id:b3\n  notebook: id:nb\n  key: "N9"'), JSON.stringify(written));

/** Replace the cell's text, wait for its verdict, and say what ran. */
const typeInCell = (text) => page.evaluate(async (text) => {
  const root = globalThis.__notebook.querySelector("prose-editor").shadowRoot;
  const editor = root.querySelector(".md-code-block code-editor");
  const before = globalThis.__evaluations.length;
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
  await new Promise((r) => setTimeout(r, 300));
  const slot = editor.closest(".md-code-block").querySelector(".notebook-cell-result");
  return {
    runs: globalThis.__evaluations.slice(before).filter((e) => e.notation === text).map((e) => e.transact),
    bolt: !!slot.querySelector(".evaluate-play"),
    result: slot.innerHTML,
  };
}, text);

await page.evaluate(() => {
  globalThis.__answer = (notation) => notation.includes("bad")
    ? { matches_after: [] }
    : { matches_after: [{ label: "person", results: [{ this: "id:alice", fields: { name: "Alice" } }] }] };
});
const query = await typeInCell("person:\n  name: ?name");
check("a query cell draws each result by its concept's display, in the notebook's context",
  JSON.stringify(query.runs) === "[false]" && query.result.includes('<tonk-display entity="id:alice" model="person" with="main@did:key:zSpace">'),
  JSON.stringify(query));
const broken = await typeInCell("bad: 1");
check("a cell with errors does not run", broken.runs.length === 0, JSON.stringify(broken));
const assertion = await typeInCell('person!:\n  name: "Bob"');
check("an asserting cell never runs on typing, and offers the bolt", assertion.runs.length === 0 && assertion.bolt, JSON.stringify(assertion));
const bolted = await page.evaluate(async () => {
  const root = globalThis.__notebook.querySelector("prose-editor").shadowRoot;
  const before = globalThis.__evaluations.length;
  root.querySelector(".evaluate-play").click();
  await globalThis.__wait(() => globalThis.__evaluations.length > before, 3_000);
  await new Promise((r) => setTimeout(r, 100));
  return globalThis.__evaluations.slice(before).map((e) => [e.transact, e.notation]);
});
check("the bolt commits the cell", JSON.stringify(bolted) === JSON.stringify([[true, 'person!:\n  name: "Bob"']]), JSON.stringify(bolted));
const ran = await page.evaluate(async () => {
  globalThis.__answer = (notation, transact) => {
    if (transact) throw new Error("nope <here>");
    return { matches_after: [] };
  };
  const root = globalThis.__notebook.querySelector("prose-editor").shadowRoot;
  const editor = root.querySelector(".md-code-block code-editor");
  const before = globalThis.__evaluations.length;
  editor.dispatchEvent(new CustomEvent("run", { bubbles: true, composed: true, detail: { value: editor.value } }));
  const slot = editor.closest(".md-code-block").querySelector(".notebook-cell-result");
  await globalThis.__wait(() => slot.querySelector(".nb-out--error"), 3_000);
  return { runs: globalThis.__evaluations.slice(before).map((e) => e.transact), error: slot.querySelector(".nb-out--error")?.textContent };
});
check("Cmd/Ctrl+Enter commits the cell, and a refusal shows on it", JSON.stringify(ran.runs) === "[true]" && ran.error === "!nope <here>",
  JSON.stringify(ran));
await page.evaluate(() => { globalThis.__answer = () => ({ matches_before: [], matches_after: [] }); });

const marked = await page.evaluate(async () => {
  const prose = globalThis.__notebook.querySelector("prose-editor");
  const view = prose.editor.view;
  view.focus();
  let at = null;
  view.state.doc.descendants((node, pos) => {
    if (at === null && node.isText && node.text.includes("intro paragraph")) at = pos + 2;
  });
  const { TextSelection } = { TextSelection: view.state.selection.constructor };
  view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, at)));
  globalThis.__notebook.mark();
  await new Promise((r) => setTimeout(r, 100));
  return [...prose.shadowRoot.querySelector(".md-doc").children].map((node) => getComputedStyle(node).boxShadow !== "none");
});
check("the block the caret is in is banded whole, heading and all",
  JSON.stringify(marked.slice(0, 3)) === JSON.stringify([true, true, false]), JSON.stringify(marked));

const unrouted = await page.evaluate(async () => {
  const el = document.createElement("notebook-editor");
  document.body.append(el);
  await new Promise((r) => setTimeout(r, 50));
  const before = el.querySelector(".notebook-error")?.textContent;
  el.setAttribute("with", "main@did:key:zSpace");
  await globalThis.__wait(() => el.querySelector("prose-editor"), 3_000);
  const after = { error: !!el.querySelector(".notebook-error"), editor: !!el.querySelector("prose-editor") };
  el.remove();
  return { before, after };
});
check("without a with it says so, and mounts once one arrives",
  unrouted.before?.startsWith("no repository in context") && !unrouted.after.error && unrouted.after.editor, JSON.stringify(unrouted));

// ---------------------------------------------------------------------------
// <notebook-index>
// ---------------------------------------------------------------------------

await define(page, "notebook.yaml", "notebook-index");
const index = await page.evaluate(async () => {
  const el = document.createElement("notebook-index");
  el.className = "notebook-index";
  el.innerHTML =
    '<notebook-save style="display: block;"><notebook-editor draft with="main@did:key:zSpace"></notebook-editor></notebook-save>' +
    '<div class="notebook-index"><a class="notebook-index__item" href="/notebook/id:nb"><span class="notebook-index__title">Scratch</span></a></div>' +
    '<div class="notebook-index"><a class="notebook-index__item" href="/notebook/id:other"><span class="notebook-index__title">Groceries</span></a></div>';
  document.body.append(el);
  globalThis.__index = el;
  const prose = await globalThis.__wait(() => el.querySelector("notebook-editor[draft] prose-editor")?.editor && el.querySelector("prose-editor"));
  await globalThis.__wait(() => prose.candidates?.length === 2, 3_000);
  const before = globalThis.__evaluations.length;
  const view = prose.editor.view;
  view.focus();
  view.dispatch(view.state.tr.insertText("Scr", 1));
  const panel = el.querySelector(".notebook-switcher__panel");
  await globalThis.__wait(() => !panel.hidden, 3_000);
  await new Promise((r) => setTimeout(r, 800));
  return {
    switcher: prose.hasAttribute("switcher"),
    candidates: prose.candidates,
    rows: [...panel.querySelectorAll(".notebook-switcher__row")].map((row) => ({
      name: row.querySelector(".notebook-switcher__name").textContent,
      marked: [...row.querySelectorAll("mark")].map((m) => m.textContent).join(""),
      verb: row.querySelector(".notebook-switcher__verb").textContent,
      active: row.classList.contains("notebook-switcher__row--active"),
    })),
    afterDraft: panel.previousElementSibling?.tagName,
    commits: globalThis.__evaluations.slice(before).filter((e) => e.transact).length,
    styled: !!document.getElementById("notebook-index-style"),
  };
});
check("the index hands its draft the notebooks the directory rendered",
  index.switcher && JSON.stringify(index.candidates) === JSON.stringify([{ title: "Scratch", href: "/notebook/id:nb" }, { title: "Groceries", href: "/notebook/id:other" }]),
  JSON.stringify(index));
check("typing a title suggests the notebooks it matches, marked, beside creating it",
  index.rows.some((row) => row.name === "Scratch" && row.verb === "open" && row.marked === "Scr")
    && index.rows.some((row) => row.verb === "create") && index.afterDraft === "NOTEBOOK-SAVE" && index.styled, JSON.stringify(index));
check("a draft commits nothing", index.commits === 0, JSON.stringify(index));

const created = await page.evaluate(async () => {
  const el = globalThis.__index;
  const before = globalThis.__evaluations.length;
  el.querySelector("prose-editor").dispatchEvent(new CustomEvent("create", {
    bubbles: true, composed: true, detail: { title: "Plans", document: "# Plans\n\nfirst thing" },
  }));
  await globalThis.__wait(() => globalThis.__navigated.length > 0, 3_000);
  el.querySelector("prose-editor").dispatchEvent(new CustomEvent("switch", {
    bubbles: true, composed: true, detail: { title: "Scratch", href: "/notebook/id:nb" },
  }));
  return { evaluations: globalThis.__evaluations.slice(before), navigated: globalThis.__navigated.slice() };
});
const entity = created.navigated[0]?.match(/^\/notebook\/(notebook:[0-9a-f-]+)$/)?.[1];
check("creating writes the title and the draft's blocks in one commit, then opens it",
  created.evaluations.length === 1 && created.evaluations[0].transact && !!entity
    && created.evaluations[0].notation.startsWith(`notebook/named!:\n  this: ${entity}\n  title: "Plans"\n`)
    && created.evaluations[0].notation.includes("source: |-\n    first thing") && created.evaluations[0].notation.includes("source: |-\n    # Plans"),
  JSON.stringify(created));
check("picking a suggestion opens it", created.navigated[1] === "/notebook/id:nb", JSON.stringify(created.navigated));

// ---------------------------------------------------------------------------
// Without the editors
// ---------------------------------------------------------------------------

const bare = await open();
await define(bare, "notebook.yaml", "notebook-editor");
const missing = await bare.evaluate(async () => {
  globalThis.__installed = new Set();
  const needed = [];
  document.addEventListener("tonk-element-needed", (event) => needed.push(event.detail.tag));
  const el = document.createElement("notebook-editor");
  el.setAttribute("with", "main@did:key:zSpace");
  document.body.append(el);
  const notice = await globalThis.__wait(() => {
    const found = el.querySelector(".notebook-requires");
    return found && !found.hidden && found.textContent.includes("code-editor") ? found : null;
  }, 3_000);
  return { text: notice?.textContent, needed, editor: !!el.querySelector("prose-editor") };
});
check("without the editor components it says which to install, and asks for the editor",
  missing.text?.includes("prose-editor") && missing.text?.includes("code-editor") && missing.needed.includes("prose-editor") && !missing.editor,
  JSON.stringify(missing));
await define(bare, "code-editor.yaml", "code-editor");
await define(bare, "prose-editor.yaml", "prose-editor");
const arrived = await bare.evaluate(async () => {
  const el = document.querySelector("notebook-editor");
  const prose = await globalThis.__wait(() => el.querySelector("prose-editor")?.editor && el.querySelector("prose-editor"), 5_000);
  return { notice: el.querySelector(".notebook-requires")?.hidden, editor: !!prose };
});
check("once they are installed the notice goes and the editor mounts", arrived.notice === true && arrived.editor, JSON.stringify(arrived));

check("no errors on the pages", problems.length === 0, problems.join("\n"));
await browser.close();

for (const { name, ok, detail } of results) {
  console.log(`${ok ? "ok  " : "FAIL"} ${name}${detail ? `\n       ${detail}` : ""}`);
}
const failed = results.filter((r) => !r.ok).length;
console.log(`\n${results.length - failed} passed, ${failed} failed`);
process.exit(failed ? 1 : 0);
