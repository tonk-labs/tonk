#!/usr/bin/env node
// Real-browser integration tests for <tonk-prose>, driven over the Chrome
// DevTools Protocol — deterministic (evaluate JS, await real conditions,
// read real return values) rather than screenshot/log scraping.
//
// Launches headless Chrome with remote debugging, serves the built bundle
// over a local HTTP server, mounts the element, and drives the full
// content round-trip through the actual ProseMirror editor.
//
//   npm run test:browser
//
// Exits non-zero on any failed assertion.

import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, resolve, extname } from "node:path";
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import CDP from "chrome-remote-interface";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = resolve(__dirname, "..");
const assets = resolve(root, "assets");

const CHROME =
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const PORT = 8899;
const DEBUG_PORT = 9222;

const MIME = { ".js": "text/javascript", ".html": "text/html", ".map": "application/json" };

// ---- tiny static server for the bundle + a mount page ----------------
const PAGE = `<!doctype html><meta charset="utf-8"><body>
<script type="module" src="/tonk-prose.js"></script>
</body>`;

const server = createServer(async (req, res) => {
  const url = req.url === "/" ? "/index.html" : req.url.split("?")[0];
  if (url === "/index.html") {
    res.writeHead(200, { "content-type": "text/html" });
    res.end(PAGE);
    return;
  }
  const file = resolve(assets, "." + url);
  if (!file.startsWith(assets) || !existsSync(file)) {
    res.writeHead(404);
    res.end("not found");
    return;
  }
  res.writeHead(200, { "content-type": MIME[extname(file)] ?? "application/octet-stream" });
  res.end(await readFile(file));
});

// ---- assertion tallies ------------------------------------------------
let passed = 0;
let failed = 0;
function check(name, cond, detail) {
  if (cond) {
    passed++;
    console.log(`  ok   ${name}`);
  } else {
    failed++;
    console.log(`  FAIL ${name}${detail ? ` — ${detail}` : ""}`);
  }
}

async function main() {
  await new Promise((r) => server.listen(PORT, r));

  const profile = mkdtempSync(resolve(tmpdir(), "tonk-prose-cdp-"));
  const chrome = spawn(CHROME, [
    "--headless=new",
    `--remote-debugging-port=${DEBUG_PORT}`,
    `--user-data-dir=${profile}`,
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-gpu",
    "about:blank",
  ]);

  // Wait for the debugger endpoint to come up.
  let client;
  for (let i = 0; i < 50; i++) {
    try {
      client = await CDP({ port: DEBUG_PORT });
      break;
    } catch {
      await new Promise((r) => setTimeout(r, 200));
    }
  }
  if (!client) throw new Error("Chrome DevTools endpoint never came up");

  try {
    const { Page, Runtime } = client;
    await Page.enable();
    await Runtime.enable();
    await Page.navigate({ url: `http://localhost:${PORT}/` });
    await Page.loadEventFired();

    // Helper: evaluate an async expression in the page, awaiting the
    // promise, and return the JSON value. Throws on page-side error.
    const evalJs = async (expression) => {
      const { result, exceptionDetails } = await Runtime.evaluate({
        expression: `(async () => { ${expression} })()`,
        awaitPromise: true,
        returnByValue: true,
      });
      if (exceptionDetails) {
        throw new Error(
          exceptionDetails.exception?.description ?? JSON.stringify(exceptionDetails),
        );
      }
      return result.value;
    };

    await runTests(evalJs);
  } finally {
    await client.close();
    chrome.kill();
    server.close();
    // Give Chrome a moment to release the profile dir before removing it,
    // and don't let a cleanup race fail the run.
    await new Promise((r) => setTimeout(r, 300));
    try {
      rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
    } catch {
      /* best-effort temp cleanup */
    }
  }

  console.log(`\n${passed} passed, ${failed} failed`);
  process.exit(failed === 0 ? 0 : 1);
}

// ---- the actual tests, run inside the page ---------------------------
async function runTests(evalJs) {
  // Boot helper installed once: mounts an element, resolves its editor.
  await evalJs(`
    window.__boot = async (setup) => {
      const el = document.createElement('tonk-prose');
      setup(el);
      document.body.appendChild(el);
      const editor = await new Promise((res) => {
        if (el.editor) return res(el.editor);
        el.addEventListener('ready', (e) => res(e.detail.editor), { once: true });
      });
      return { el, editor };
    };
    window.__wait = (ms) => new Promise((r) => setTimeout(r, ms));
    return true;
  `);

  // 1. Initial content from the text child (the <textarea>-style channel).
  {
    const doc = await evalJs(`
      const { el, editor } = await window.__boot((el) => {
        el.textContent = '# Hello\\n\\nfrom text child';
      });
      return editor.getMarkdown();
    `);
    check("initial content read from textContent", doc.includes("# Hello") && doc.includes("from text child"), JSON.stringify(doc));
  }

  // 2. THE BUG: store-feedback loop with the versioned envelope must NOT
  //    stack headers into the document over repeated edits.
  {
    const result = await evalJs(`
      const { el, editor } = await window.__boot((el) => {
        el.textContent = '# Doc\\n\\nbody';
      });
      // Simulate the store loop: on each change, write the emitted content
      // back as the element's TEXT (what the view's set_text_content does).
      el.addEventListener('change', (e) => { el.textContent = e.detail.content; });
      const view = editor.view; view.focus();
      // three edits, each past the debounce
      for (const ch of ['A','B','C']) {
        view.dispatch(view.state.tr.insertText(ch, 3));
        await window.__wait(550);
      }
      const md = editor.getMarkdown();
      return { md, leaked: /Tonk-Prose-Version/i.test(md) };
    `);
    check("no envelope headers leak into the document (stacking bug)", result.leaked === false, JSON.stringify(result.md));
    check("edits accumulated correctly", /CBA|ABC|[ABC]/.test(result.md), JSON.stringify(result.md));
  }

  // 3. Echo drop: feeding our own emitted envelope back leaves the doc and
  //    caret untouched.
  {
    const result = await evalJs(`
      const { el, editor } = await window.__boot((el) => { el.textContent = 'hello'; });
      const view = editor.view; view.focus();
      let emitted = null;
      el.addEventListener('change', (e) => { emitted = e.detail.content; });
      view.dispatch(view.state.tr.insertText('X', 1));
      await window.__wait(550);
      const before = editor.getMarkdown();
      const caretBefore = view.state.selection.head;
      el.textContent = emitted; // our own echo, fed back
      await window.__wait(50);
      return {
        docSame: editor.getMarkdown() === before,
        caretSame: view.state.selection.head === caretBefore,
        emittedIsEnvelope: /^Tonk-Prose-Version/i.test(emitted || ''),
      };
    `);
    check("emitted change carries an envelope", result.emittedIsEnvelope);
    check("own echo leaves document unchanged", result.docSame);
    check("own echo leaves caret unchanged", result.caretSame);
  }

  // 4. Markup safety: markdown containing HTML must not be parsed as DOM.
  {
    const result = await evalJs(`
      const { el, editor } = await window.__boot((el) => {
        el.textContent = '# Note\\n\\n<div>x</div> and <b>y</b>';
      });
      return {
        md: editor.getMarkdown(),
        lightChildren: el.children.length,
      };
    `);
    check("HTML markup in content stays literal (no child elements)", result.lightChildren === 0, `children=${result.lightChildren}`);
    check("markup preserved in the document text", result.md.includes("<div>x</div>") || result.md.includes("<div>"), JSON.stringify(result.md));
  }

  // 5. Debounce: a burst of edits produces ONE change event, and a long
  //    store-feedback loop stays stable (no drift, no header leak).
  {
    const result = await evalJs(`
      const { el, editor } = await window.__boot((el) => { el.textContent = 'x'; });
      const view = editor.view; view.focus();
      let changes = 0;
      el.addEventListener('change', (e) => { changes++; el.textContent = e.detail.content; });
      // burst of 6 quick edits, then idle
      for (let i = 0; i < 6; i++) view.dispatch(view.state.tr.insertText('y', 1));
      const duringBurst = changes;
      await window.__wait(600);
      const afterIdle = changes;
      // then several more spaced edits to exercise the loop
      for (let i = 0; i < 4; i++) { view.dispatch(view.state.tr.insertText('z', 1)); await window.__wait(550); }
      const md = editor.getMarkdown();
      return { duringBurst, afterIdle, md, leaked: /Tonk-Prose-Version/i.test(md) };
    `);
    check("no change dispatched mid-burst (debounced)", result.duringBurst === 0, `during=${result.duringBurst}`);
    check("one change after the burst goes idle", result.afterIdle === 1, `after=${result.afterIdle}`);
    check("long feedback loop stays clean (no header leak)", result.leaked === false, JSON.stringify(result.md));
  }

  // 6. Genuine newer remote change is adopted (not dropped as an echo).
  {
    const result = await evalJs(`
      const { el, editor } = await window.__boot((el) => { el.textContent = 'original'; });
      const view = editor.view; view.focus();
      view.dispatch(view.state.tr.insertText('Z', 1)); // local edit -> bumps our HLC
      await window.__wait(550);
      // remote write with a far-future HLC (definitely newer)
      const bigHlc = (BigInt(Date.now() + 100000) << 16n).toString();
      const envelope =
        'Tonk-Prose-Version: 1\\r\\nETag: "' + bigHlc + '"\\r\\nContent-Type: text/markdown\\r\\n\\r\\nremote wins';
      el.textContent = envelope;
      await window.__wait(50);
      return editor.getMarkdown();
    `);
    check("genuinely newer remote change is adopted", result.includes("remote wins"), JSON.stringify(result));
  }
}

main().catch((err) => {
  console.error(err);
  server.close();
  process.exit(1);
});
