// The library's <table-editor>, driven in a real Chromium the way a space
// renders it: its definition read out of `table.yaml` (each
// `!include/text` inlined, the engine among them), handed to the real
// element runtime (`rust/tonk-display/assets/element-runtime.js`). First
// standalone, its text the workbook; then over claim rows shaped like the
// table views render, emitting the events `table.yaml` binds to commands.
//
// Run it (after `npm run build` here):
//
//   node rust/tonk-table/tests/library-element.mjs
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

/** Every DOM event type an `event!:` in `table.yaml` listens for. */
const bound = [...readFileSync(join(library, "table.yaml"), "utf8").matchAll(/^event!: &\S+\n  type: "([a-z]+)"/gm)]
  .map((m) => m[1]);

const results = [];
const check = (name, ok, detail = "") => results.push({ name, ok, detail: ok ? "" : detail });

const browser = await chromium.launch({
  args: ["--no-proxy-server"],
  ...(process.env.CHROMIUM_BIN ? { executablePath: process.env.CHROMIUM_BIN } : {}),
});
const page = await browser.newPage();
const problems = [];
page.on("console", (m) => { if (m.type() === "error") problems.push(m.text()); });
page.on("pageerror", (e) => problems.push(String(e)));
await page.setContent("<!doctype html><html><head></head><body></body></html>");
await page.addScriptTag({ content: RUNTIME });
await page.addScriptTag({ type: "module", content: module("table-editor", definition("table.yaml", "table-editor")) });
await page.waitForFunction(() => customElements.get("table-editor"));

/** Wait for `selector`'s grid to be mounted. */
const ready = (selector) =>
  page.waitForFunction((selector) => !!document.querySelector(selector)?.grid, selector, { timeout: 20000 });

// 1. Standalone: the text is the workbook, and formulas compute.
await page.evaluate(() => {
  const el = document.createElement("table-editor");
  el.id = "solo";
  el.textContent = "a,b,total\n1,2,=A2+B2";
  document.body.append(el);
});
await ready("#solo");
const solo = await page.evaluate(() => {
  const el = document.querySelector("#solo");
  return {
    total: el.grid.getCell("C2")?.value,
    formula: el.grid.getCell("C2")?.content,
    painted: el.shadowRoot.querySelectorAll("td").length,
    value: el.value,
  };
});
check("mounts the grid in its shadow root", solo.painted > 0, JSON.stringify(solo));
check("a formula in the text computes", solo.total === "3" && solo.formula === "=A2+B2", JSON.stringify(solo));
check("value reads the active sheet as CSV", solo.value.startsWith("a,b,total"), JSON.stringify(solo));

// 2. An edit fires one debounced `change`, and its echo is dropped.
const edit = await page.evaluate(async () => {
  const el = document.querySelector("#solo");
  const seen = [];
  el.addEventListener("change", (e) => seen.push(e.detail));
  el.grid.setCell("A2", "40");
  el.grid.setCell("B2", "2");
  await new Promise((r) => setTimeout(r, 600));
  const total = el.grid.getCell("C2")?.value;
  const version = el.version;
  // The store writes the envelope back as text: the element's own echo.
  const reloads = [];
  const load = el.grid.load.bind(el.grid);
  el.grid.load = (source) => { reloads.push(source.kind); load(source); };
  el.textContent = seen.at(-1)?.content ?? "";
  await new Promise((r) => setTimeout(r, 50));
  return { count: seen.length, value: seen.at(-1)?.value, content: seen.at(-1)?.content ?? "", total, version, reloads };
});
check("edits fire one change after the idle gap", edit.count === 1, JSON.stringify(edit));
check("the edit recomputes", edit.total === "42", JSON.stringify(edit));
check("change carries CSV and a versioned envelope",
  edit.value?.includes("40,2") && /^Tonk-Table-Version: 1\r\nETag: "\d+"/.test(edit.content) && edit.version !== "0",
  JSON.stringify({ ...edit, content: edit.content.slice(0, 80) }));
check("its own echo does not reload the grid", edit.reloads.length === 0, JSON.stringify(edit.reloads));

// 3. A newer bare write is adopted.
const adopted = await page.evaluate(async () => {
  const el = document.querySelector("#solo");
  el.value = "x,y\n3,=A2*10";
  await new Promise((r) => setTimeout(r, 50));
  return el.grid.getCell("B2")?.value;
});
check("a bare CSV write is adopted", adopted === "30", String(adopted));

// 4. Claims: the rows the views render feed the grid.
await page.evaluate(() => {
  document.body.insertAdjacentHTML("beforeend", `
    <table-editor id="book" subject="id:book">
      <div hidden>
        <div class="table-sheet-row" subject="id:sheet1" data-table="id:book" data-name="Sheet1" data-order="m"></div>
        <div class="table-cell-row" subject="id:a1" data-sheet="id:sheet1" data-at="A1" data-content="2"></div>
        <div class="table-cell-row" subject="id:b1" data-sheet="id:sheet1" data-at="B1" data-content="3.5"></div>
        <div class="table-cell-row" subject="id:c1" data-sheet="id:sheet1" data-at="C1" data-content="=A1*B1"></div>
      </div>
    </table-editor>`);
});
await ready("#book");
await page.evaluate(() => new Promise((r) => setTimeout(r, 100)));
const claims = await page.evaluate(() => document.querySelector("#book").grid.getCell("C1")?.value);
check("claim rows render and compute", claims === "7", String(claims));

// 5. Edits leave as the typed events table.yaml binds to commands.
const events = await page.evaluate(async (types) => {
  const el = document.querySelector("#book");
  const seen = [];
  for (const type of types) el.addEventListener(type, (e) => seen.push({ type, detail: e.detail }));
  el.grid.setCell("A1", "4");
  el.grid.setCell("A2", "=C1+1");
  await new Promise((r) => setTimeout(r, 50));
  return { seen, c1: el.grid.getCell("C1")?.value, a2: el.grid.getCell("A2")?.value };
}, ["createcell", "editcell", "clearcell", "createsheet", "renamesheet", "createcolumn", "resizecolumn", "createrow", "resizerow", "change"]);
const edited = events.seen.find((e) => e.type === "editcell");
const created = events.seen.find((e) => e.type === "createcell");
check("editing a claimed cell fires editcell for it",
  edited?.detail.editCell === "id:a1" && edited?.detail.editContent === "4", JSON.stringify(events.seen));
check("writing an empty cell fires createcell on its sheet",
  created?.detail.cellSheet === "id:sheet1" && created?.detail.cellAt === "A2" && created?.detail.cellContent === "=C1+1"
    && Number.isInteger(created?.detail.time),
  JSON.stringify(events.seen));
check("claims mode fires no change", !events.seen.some((e) => e.type === "change"), JSON.stringify(events.seen));
check("the edit recomputes optimistically", events.c1 === "14" && events.a2 === "15", JSON.stringify(events));
check("every event it fired is one table.yaml binds",
  events.seen.every((e) => bound.includes(e.type)), JSON.stringify({ bound, fired: events.seen.map((e) => e.type) }));

// 6. The store's answer arrives as rows, and a remote edit repaints.
const remote = await page.evaluate(async () => {
  const el = document.querySelector("#book");
  const rows = el.querySelector("[hidden]");
  rows.querySelector('[subject="id:a1"]').dataset.content = "4";
  rows.insertAdjacentHTML("beforeend",
    `<div class="table-cell-row" subject="id:a2" data-sheet="id:sheet1" data-at="A2" data-content="=C1+1"></div>`);
  await new Promise((r) => setTimeout(r, 100));
  rows.querySelector('[subject="id:b1"]').dataset.content = "10";
  await new Promise((r) => setTimeout(r, 100));
  return { c1: el.grid.getCell("C1")?.value, a2: el.grid.getCell("A2")?.value };
});
check("a remote edit to a row repaints the grid", remote.c1 === "40" && remote.a2 === "41", JSON.stringify(remote));

// 7. Removal tears the grid down.
const removed = await page.evaluate(async () => {
  const el = document.querySelector("#solo");
  el.remove();
  await new Promise((r) => setTimeout(r, 20));
  return el.grid === null;
});
check("removing the element tears the grid down", removed);

check("no errors on the page", problems.length === 0, problems.join("\n"));
await browser.close();

for (const { name, ok, detail } of results) {
  console.log(`${ok ? "ok  " : "FAIL"} ${name}${detail ? `\n       ${detail}` : ""}`);
}
const failed = results.filter((r) => !r.ok).length;
console.log(`\n${results.length - failed} passed, ${failed} failed`);
process.exit(failed ? 1 : 0);
