// The library's <code-editor>, driven in a real Chromium the way a space
// renders it: its definition read out of `code-editor.yaml` (each
// `!include/text` inlined), handed to the real element runtime
// (`rust/tonk-display/assets/element-runtime.js`), and its languages
// answered by a stand-in host from records shaped like the space's.
// Then <prose-editor> embeds it in a fenced code block, and an edit
// there reaches the markdown. Last, an editor with a `source` talks to a
// stand-in language server behind `fetch`.
//
// Run it (after `npm run build` here):
//
//   node rust/tonk-code/tests/library-element.mjs
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

/** The `element!:` named `tag` in `file`, as the registry hands it to
 *  `defineTonkElement`: each dictionary a map of key to source. Enough of
 *  YAML for how the library writes elements: `key: !include/text ./path`
 *  and `key: |` block scalars, two levels under the element. */
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

/** The language records a space with `language/python` installed holds:
 *  `this`, its words and pack, as the YAML generates them. */
function records(...files) {
  const out = [];
  for (const file of files) {
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
  }
  return out;
}

const results = [];
const check = (name, ok, detail = "") => results.push({ name, ok, detail: ok ? "" : detail });

const browser = await chromium.launch({ args: ["--no-proxy-server"] });
const page = await browser.newPage();
const problems = [];
page.on("console", (m) => { if (m.type() === "error") problems.push(m.text()); });
page.on("pageerror", (e) => problems.push(String(e)));
await page.setContent("<!doctype html><html><head></head><body></body></html>");

// The stand-in host: answers `tonk-query` and `tonk-subscribe` for the
// language request from `branch`, and pushes a frame to every subscriber
// when `install` adds records.
await page.evaluate((initial) => {
  const branch = (globalThis.__branch = initial);
  const subscribers = (globalThis.__subscribers = new Set());
  const answer = (query) => {
    const word = query?.terms?.word;
    return branch
      .filter((r) => r.words.includes(word) && r.source)
      .map((r) => ({ this: r.this, fields: { this: r.this, word, source: r.source } }));
  };
  document.addEventListener("tonk-query", (event) => {
    event.preventDefault();
    event.detail.result = Promise.resolve(answer(event.detail.query));
  });
  document.addEventListener("tonk-subscribe", (event) => {
    event.preventDefault();
    const consumer = event.composedPath()[0];
    subscribers.add(consumer);
    event.detail.subscription = { cancel: () => subscribers.delete(consumer) };
  });
  globalThis.__install = (more) => {
    branch.push(...more);
    for (const consumer of subscribers) consumer.update?.({}, { tag: "language" });
  };
}, records("code-editor.yaml"));

await page.addScriptTag({ content: RUNTIME });
const execute = (source) => page.addScriptTag({ type: "module", content: source });
await execute(module("code-editor", definition("code-editor.yaml", "code-editor")));
await page.waitForFunction(() => customElements.get("code-editor"));

const settle = () => page.evaluate(() => new Promise((r) => setTimeout(r, 150)));
/** Tokens drawn with a highlight class, in `selector`'s editor. */
const highlighted = (selector) =>
  page.evaluate((selector) => {
    const host = document.querySelector(selector);
    return [...(host?.shadowRoot?.querySelectorAll(".cm-line span") ?? [])].length;
  }, selector);

// 1. A shipped language, named by one of its other words.
await page.evaluate(() => {
  document.body.insertAdjacentHTML(
    "beforeend",
    `<code-editor id="dialog" language="dialog" value="person!:\n  this: id:alice\n  name: &quot;Alice&quot;"></code-editor>`,
  );
});
await settle();
check("mounts a CodeMirror view in its shadow root",
  await page.evaluate(() => !!document.querySelector("#dialog").view && !!document.querySelector("#dialog").shadowRoot.querySelector(".cm-editor")));
check("value reads the document", await page.evaluate(() => document.querySelector("#dialog").value.startsWith("person!:")));
check("a language the editor ships highlights", (await highlighted("#dialog")) > 0, "no highlighted tokens");

// 2. A language the space lacks is plain text, and redraws once installed.
await page.evaluate(() => {
  document.body.insertAdjacentHTML("beforeend", `<code-editor id="py" language="py" value="def f(x):\n    return 'x'"></code-editor>`);
});
await settle();
check("a language the space lacks is plain text", (await highlighted("#py")) === 0, "highlighted with no record");
await page.evaluate((more) => globalThis.__install(more), records("language/python.yaml"));
await settle();
check("installing the language redraws the open block", (await highlighted("#py")) > 0, "still plain after install");

// 3. Edits: `change` for the user's, none for a `value` write.
const edits = await page.evaluate(async () => {
  const el = document.querySelector("#py");
  const seen = [];
  el.addEventListener("change", (e) => seen.push(e.detail.value));
  el.value = "x = 1";
  const quiet = seen.length;
  el.view.dispatch({ changes: { from: el.view.state.doc.length, insert: "\ny = 2" }, userEvent: "input" });
  return { quiet, seen, value: el.value };
});
check("a value write fires no change", edits.quiet === 0, JSON.stringify(edits));
check("an edit fires change with the document", edits.seen.at(-1) === "x = 1\ny = 2", JSON.stringify(edits));

// 4. Inside <prose-editor>: a fenced block is a highlighted <code-editor>,
// and typing in it reaches the markdown.
await execute(module("prose-editor", definition("prose-editor.yaml", "prose-editor")));
await page.waitForFunction(() => customElements.get("prose-editor"));
await page.evaluate(() => {
  const prose = document.createElement("prose-editor");
  prose.id = "prose";
  prose.textContent = "# Notes\n\n```python\nprint('hi')\n```\n";
  document.body.append(prose);
});
await settle();
const embedded = await page.evaluate(() => {
  const prose = document.querySelector("#prose");
  const code = prose.shadowRoot.querySelector("code-editor");
  return code ? { language: code.getAttribute("language"), value: code.value, spans: code.shadowRoot.querySelectorAll(".cm-line span").length } : null;
});
check("a fenced block embeds <code-editor>", embedded?.value === "print('hi')", JSON.stringify(embedded));
check("the embedded block is highlighted", embedded?.spans > 0, JSON.stringify(embedded));
const markdown = await page.evaluate(async () => {
  const prose = document.querySelector("#prose");
  const code = prose.shadowRoot.querySelector("code-editor");
  code.view.dispatch({ changes: { from: code.view.state.doc.length, insert: "\nprint('there')" }, userEvent: "input" });
  await new Promise((r) => setTimeout(r, 50));
  return prose.value;
});
check("an edit in the block reaches the markdown", markdown.includes("```python\nprint('hi')\nprint('there')\n```"), JSON.stringify(markdown));

// 5. Defined after the prose editor mounted, the blocks still upgrade.
const late = await browser.newPage();
await late.setContent("<!doctype html><html><head></head><body></body></html>");
await late.addScriptTag({ content: RUNTIME });
await late.addScriptTag({ type: "module", content: module("prose-editor", definition("prose-editor.yaml", "prose-editor")) });
await late.waitForFunction(() => customElements.get("prose-editor"));
const asked = await late.evaluate(async () => {
  const tags = [];
  document.addEventListener("tonk-element-needed", (e) => tags.push(e.detail.tag));
  const prose = document.createElement("prose-editor");
  prose.textContent = "```\nplain\n```\n";
  document.body.append(prose);
  await new Promise((r) => setTimeout(r, 50));
  return { tags, plain: !prose.shadowRoot.querySelector("code-editor") };
});
await late.addScriptTag({ type: "module", content: module("code-editor", definition("code-editor.yaml", "code-editor")) });
await late.waitForFunction(() => customElements.get("code-editor"));
await late.evaluate(() => new Promise((r) => setTimeout(r, 100)));
const upgraded = await late.evaluate(() => !!document.querySelector("prose-editor").shadowRoot.querySelector("code-editor")?.view);
check("<prose-editor> asks for code-editor and stays plain until it is defined", asked.tags.includes("code-editor") && asked.plain, JSON.stringify(asked));
check("its blocks become editors once code-editor is defined", upgraded);

// 6. The language server: an editor with a `source` opens it on the
// server and hears its verdicts as `diagnostics`. The server is a stand-in
// behind `fetch`, speaking the HTTP + SSE transport the worker does, which
// finds one error in any document containing `bad`.
const lsp = await browser.newPage();
lsp.on("console", (m) => { if (m.type() === "error") problems.push(m.text()); });
lsp.on("pageerror", (e) => problems.push(String(e)));
await lsp.setContent("<!doctype html><html><head></head><body></body></html>");
await lsp.evaluate(() => {
  const encoder = new TextEncoder();
  const sent = (globalThis.__sent = []);
  const streams = new Set();
  const push = (message) => {
    for (const controller of streams) controller.enqueue(encoder.encode(`data: ${JSON.stringify(message)}\n\n`));
  };
  const verdict = (uri, version, text) => push({
    jsonrpc: "2.0",
    method: "textDocument/publishDiagnostics",
    params: {
      uri, version,
      diagnostics: text.includes("bad")
        ? [{ range: { start: { line: 0, character: 0 }, end: { line: 0, character: 3 } }, severity: 1, message: "bad" }]
        : [],
    },
  });
  window.fetch = async (url, init = {}) => {
    if (url !== "/api/language-server") return new Response("", { status: 404 });
    if (init.method !== "POST") {
      return new Response(new ReadableStream({ start: (controller) => streams.add(controller) }), {
        headers: { "content-type": "text/event-stream" },
      });
    }
    const message = JSON.parse(init.body);
    sent.push(message);
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
});
await lsp.addScriptTag({ content: RUNTIME });
await lsp.addScriptTag({ type: "module", content: module("code-editor", definition("code-editor.yaml", "code-editor")) });
await lsp.waitForFunction(() => customElements.get("code-editor"));

const opened = await lsp.evaluate(async () => {
  const frames = [];
  const announced = [];
  document.addEventListener("code-editor-connect", (e) => announced.push(e.detail));
  const el = document.createElement("code-editor");
  el.setAttribute("language", "dialog-yaml");
  el.setAttribute("source", "tonk-buffer:///did:key:zSpace/main/scratch-0");
  el.setAttribute("value", "bad: 1");
  el.addEventListener("diagnostics", (e) => frames.push({ count: e.detail.count, errorCount: e.detail.errorCount, value: e.detail.value }));
  document.body.append(el);
  // The session waits up to 2s for a service worker this page never gets.
  for (let i = 0; i < 80 && frames.length === 0; i++) await new Promise((r) => setTimeout(r, 50));
  const open = globalThis.__sent.find((m) => m.method === "textDocument/didOpen")?.params.textDocument;
  return { frames, announced, open, errorCount: el.errorCount };
});
check("an editor with a source announces it",
  opened.announced.length === 1 && opened.announced[0].source === "tonk-buffer:///did:key:zSpace/main/scratch-0"
    && opened.announced[0].language === "dialog-yaml" && opened.announced[0].url === "/api/language-server",
  JSON.stringify(opened.announced));
check("it opens its source on the language server under its language",
  opened.open?.uri === "tonk-buffer:///did:key:zSpace/main/scratch-0" && opened.open?.languageId === "dialog-yaml",
  JSON.stringify(opened.open));
check("a frame from the server fires diagnostics with its errors",
  opened.frames.length === 1 && opened.frames[0].errorCount === 1 && opened.frames[0].value === "bad: 1",
  JSON.stringify(opened.frames));
check("errorCount reads the latest frame", opened.errorCount === 1, JSON.stringify(opened));

const fixed = await lsp.evaluate(async () => {
  const el = document.querySelector("code-editor");
  const frames = [];
  el.addEventListener("diagnostics", (e) => frames.push(e.detail.errorCount));
  el.view.dispatch({ changes: { from: 0, to: el.view.state.doc.length, insert: "good: 1" }, userEvent: "input" });
  for (let i = 0; i < 80 && frames.length === 0; i++) await new Promise((r) => setTimeout(r, 50));
  return { frames, errorCount: el.errorCount };
});
check("an edit the server clears fires a clean frame", fixed.frames.at(-1) === 0 && fixed.errorCount === 0, JSON.stringify(fixed));

const shared = await lsp.evaluate(async () => {
  const el = document.createElement("code-editor");
  el.setAttribute("source", "tonk-buffer:///did:key:zSpace/main/scratch-1");
  el.setAttribute("value", "bad");
  let frame = null;
  el.addEventListener("diagnostics", (e) => { frame = e.detail.errorCount; });
  document.body.append(el);
  for (let i = 0; i < 80 && frame === null; i++) await new Promise((r) => setTimeout(r, 50));
  return {
    frame,
    initialized: globalThis.__sent.filter((m) => m.method === "initialize").length,
    language: globalThis.__sent.filter((m) => m.method === "textDocument/didOpen").at(-1)?.params.textDocument.languageId,
  };
});
check("editors share one session", shared.frame === 1 && shared.initialized === 1, JSON.stringify(shared));
check("an editor with no language opens as plaintext", shared.language === "plaintext", JSON.stringify(shared));

const closed = await lsp.evaluate(async () => {
  const el = document.querySelector("code-editor");
  const withdrawn = [];
  el.addEventListener("code-editor-disconnect", (e) => withdrawn.push(e.detail.source));
  el.remove();
  await new Promise((r) => setTimeout(r, 100));
  return {
    withdrawn,
    closed: globalThis.__sent.filter((m) => m.method === "textDocument/didClose").map((m) => m.params.textDocument.uri),
  };
});
check("removing an editor closes its document",
  closed.withdrawn[0] === "tonk-buffer:///did:key:zSpace/main/scratch-0"
    && closed.closed.includes("tonk-buffer:///did:key:zSpace/main/scratch-0"),
  JSON.stringify(closed));

const claimed = await lsp.evaluate(async () => {
  const opens = () => globalThis.__sent.filter((m) => m.method === "textDocument/didOpen").length;
  const before = opens();
  const holder = document.createElement("div");
  holder.addEventListener("code-editor-connect", (e) => e.preventDefault());
  const el = document.createElement("code-editor");
  el.setAttribute("source", "tonk-buffer:///did:key:zSpace/main/claimed");
  holder.append(el);
  document.body.append(holder);
  await new Promise((r) => setTimeout(r, 200));
  const warned = [];
  const warn = console.warn;
  console.warn = (...args) => warned.push(String(args[0]));
  el.connect({ plugin: () => [] });
  console.warn = warn;
  return { opened: opens() - before, warned };
});
check("an ancestor that claims the announcement keeps the editor off the session", claimed.opened === 0, JSON.stringify(claimed));
check("a client from another core is refused", claimed.warned.some((w) => w.includes("its own core")), JSON.stringify(claimed));

check("no errors on the page", problems.length === 0, problems.join("\n"));
await browser.close();

for (const { name, ok, detail } of results) {
  console.log(`${ok ? "ok  " : "FAIL"} ${name}${detail ? `\n       ${detail}` : ""}`);
}
const failed = results.filter((r) => !r.ok).length;
console.log(`\n${results.length - failed} passed, ${failed} failed`);
process.exit(failed ? 1 : 0);
