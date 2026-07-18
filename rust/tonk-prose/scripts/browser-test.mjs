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

  // ---- Block-structure editing (lists / blockquotes via the reparse
  //      loop). These drive real transactions through the live view and
  //      wait past the reparse debounce, then assert the resulting
  //      markdown AND caret. Helpers installed once. --------------------
  await evalJs(`
    // Content-start position of the first textblock whose text === t.
    window.__lineStart = (view, t) => {
      let pos = null;
      view.state.doc.descendants((node, p) => {
        if (pos === null && node.isTextblock && node.textContent === t) pos = p + 1;
      });
      return pos;
    };
    // Put the caret at doc position pos.
    window.__caret = (view, pos) => {
      const Sel = Object.getPrototypeOf(view.state.selection).constructor;
      view.dispatch(view.state.tr.setSelection(Sel.create(view.state.tr.doc, pos)));
    };
    // Emulate one native Backspace: delete the char before the caret.
    window.__backspace = (view) => {
      const head = view.state.selection.head;
      if (head <= 1) return;
      view.dispatch(view.state.tr.delete(head - 1, head));
    };
    return true;
  `);

  // 7. Deleting the whole "- " marker on a middle bullet lifts it out,
  //    splitting the list (one operation).
  {
    const md = await evalJs(`
      const { editor } = await window.__boot((el) => {});
      editor.setMarkdown('- one\\n\\n- two\\n\\n- three');
      const view = editor.view; view.focus();
      await window.__wait(30);
      const p = window.__lineStart(view, '- two');
      view.dispatch(view.state.tr.delete(p, p + 2)); // remove "- "
      await window.__wait(220);
      return editor.getMarkdown();
    `);
    check("delete '- ' lifts a middle bullet out of the list", md.trim() === "- one\n\ntwo\n\n- three", JSON.stringify(md));
  }

  // 8. Deleting "- [ ] " on a todo lifts it to a plain paragraph.
  {
    const md = await evalJs(`
      const { editor } = await window.__boot((el) => {});
      editor.setMarkdown('- [ ] a\\n\\n- [x] b');
      const view = editor.view; view.focus();
      await window.__wait(30);
      const p = window.__lineStart(view, '- [ ] a');
      view.dispatch(view.state.tr.delete(p, p + 6)); // remove "- [ ] "
      await window.__wait(220);
      return editor.getMarkdown();
    `);
    check("delete '- [ ] ' lifts a todo to a plain paragraph", md.trim() === "a\n\n- [x] b", JSON.stringify(md));
  }

  // 9. Deleting ">" (well, "> ") on a middle quote line splits the quote.
  {
    const md = await evalJs(`
      const { editor } = await window.__boot((el) => {});
      editor.setMarkdown('> a\\n\\n> b\\n\\n> c');
      const view = editor.view; view.focus();
      await window.__wait(30);
      const p = window.__lineStart(view, '> b');
      view.dispatch(view.state.tr.delete(p, p + 2)); // remove "> "
      await window.__wait(220);
      return editor.getMarkdown();
    `);
    check("delete '> ' splits a blockquote", md.trim() === "> a\n\nb\n\n> c", JSON.stringify(md));
  }

  // 10. After a split, the caret stays on the lifted line (not jumping to
  //     a sibling) — this is the "second backspace hits the wrong block"
  //     regression. Caret at content-col 2 of "- two"; delete "- "; caret
  //     must land at the start of the now-plain "two" paragraph.
  {
    const result = await evalJs(`
      const { editor } = await window.__boot((el) => {});
      editor.setMarkdown('- one\\n\\n- two\\n\\n- three');
      const view = editor.view; view.focus();
      await window.__wait(30);
      const p = window.__lineStart(view, '- two');
      window.__caret(view, p + 2); // caret just after "- "
      view.dispatch(view.state.tr.delete(p, p + 2));
      await window.__wait(220);
      // The caret should be inside the paragraph reading "two".
      const $h = view.state.doc.resolve(view.state.selection.head);
      return { md: editor.getMarkdown(), caretLine: $h.parent.textContent };
    `);
    check("caret rides the lifted line after a split", result.caretLine === "two", JSON.stringify(result));
  }

  // 11. Typing "- " at the start of a paragraph converts it to a bullet
  //     via the reparse loop (NOT a synchronous input rule that would
  //     make a markerless native list).
  {
    const md = await evalJs(`
      const { editor } = await window.__boot((el) => {});
      editor.setMarkdown('item');
      const view = editor.view; view.focus();
      await window.__wait(30);
      window.__caret(view, 1); // start of "item"
      view.dispatch(view.state.tr.insertText('- ', 1));
      await window.__wait(220);
      return editor.getMarkdown();
    `);
    check("typing '- ' converts a paragraph to a bullet", md.trim() === "- item", JSON.stringify(md));
  }

  // 12. A task item renders its checkbox at rest, even though its marker
  //     is the combined "- [ ] " (list + checkbox) node.
  {
    const result = await evalJs(`
      const { el, editor } = await window.__boot((el) => {});
      editor.setMarkdown('- [ ] todo\\n\\n- [x] done');
      await window.__wait(40);
      const root = el.shadowRoot;
      const boxes = [...root.querySelectorAll('.md-task-checkbox')].map(b => b.checked);
      return { count: boxes.length, boxes };
    `);
    check("task items render checkboxes (combined marker)", result.count === 2 && result.boxes[0] === false && result.boxes[1] === true, JSON.stringify(result));
  }

  // 13. Clicking a task checkbox toggles its state through the document.
  {
    const md = await evalJs(`
      const { el, editor } = await window.__boot((el) => {});
      editor.setMarkdown('- [ ] todo');
      await window.__wait(40);
      const box = el.shadowRoot.querySelector('.md-task-checkbox');
      box.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }));
      await window.__wait(220);
      return editor.getMarkdown();
    `);
    check("clicking a checkbox toggles the task state", md.trim() === "- [x] todo", JSON.stringify(md));
  }

  // 14. Deleting the "-" of the LAST list item (leaving a leading space)
  //     lifts it OUT of the list — it must not get absorbed as a second
  //     paragraph of the previous item. Regression: the lossless guard
  //     rejected the whitespace-trimmed reparse, so native backspace
  //     joined the line into the item above.
  {
    const result = await evalJs(`
      const { editor } = await window.__boot((el) => {});
      editor.setMarkdown('- one\\n\\n- two\\n\\n- three');
      const view = editor.view; view.focus();
      await window.__wait(30);
      // Delete only the "-" of "- three" (leaving "- " -> " three").
      const p = window.__lineStart(view, '- three');
      view.dispatch(view.state.tr.delete(p, p + 1));
      await window.__wait(220);
      // Structure: two-item list + a plain paragraph "three".
      const top = [];
      view.state.doc.forEach((n) => top.push(n.type.name));
      return { md: editor.getMarkdown(), top };
    `);
    check(
      "deleting '-' on the last item lifts it out (not merged above)",
      result.md.trim() === "- one\n\n- two\n\nthree" &&
        result.top.join(",") === "bullet_list,paragraph",
      JSON.stringify(result),
    );
  }

  // 15. Same for the LAST quote line: deleting ">" (leaving " ") lifts it.
  {
    const result = await evalJs(`
      const { editor } = await window.__boot((el) => {});
      editor.setMarkdown('> a\\n\\n> b\\n\\n> c');
      const view = editor.view; view.focus();
      await window.__wait(30);
      const p = window.__lineStart(view, '> c');
      view.dispatch(view.state.tr.delete(p, p + 1)); // delete ">" -> " c"
      await window.__wait(220);
      return editor.getMarkdown();
    `);
    check("deleting '>' on the last quote line lifts it out", result.trim() === "> a\n\n> b\n\nc", JSON.stringify(result));
  }

  // Enter/Shift-Enter behavior in blockquotes. Helper fires a real
  // keydown through the editor's keymap.
  await evalJs(`
    window.__key = (view, key, opts = {}) =>
      view.someProp('handleKeyDown', f =>
        f(view, new KeyboardEvent('keydown', { key, shiftKey: !!opts.shift, keyCode: key === 'Enter' ? 13 : 0 })));
    // Position the caret at the END of the textblock whose text === t.
    window.__caretEnd = (view, t) => {
      let pos = null;
      view.state.doc.descendants((node, p) => {
        if (pos === null && node.isTextblock && node.textContent === t) pos = p + 1 + node.content.size;
      });
      const Sel = Object.getPrototypeOf(view.state.selection).constructor;
      view.dispatch(view.state.tr.setSelection(Sel.create(view.state.tr.doc, pos)));
      return pos;
    };
    return true;
  `);

  // 16. Enter in a quote continues it: new line carries "> ", caret after.
  {
    const result = await evalJs(`
      const { editor } = await window.__boot((el) => {});
      editor.setMarkdown('> foo');
      const view = editor.view; view.focus();
      await window.__wait(30);
      window.__caretEnd(view, '> foo');
      window.__key(view, 'Enter');
      await window.__wait(30);
      const $h = view.state.doc.resolve(view.state.selection.head);
      // Now type "bar" and check it lands after the "> " of the new line.
      view.dispatch(view.state.tr.insertText('bar', view.state.selection.head));
      await window.__wait(220);
      return { md: editor.getMarkdown(), caretLineAfterEnter: $h.parent.textContent };
    `);
    check("Enter in a quote seeds a new '> ' line", result.caretLineAfterEnter === "> ", JSON.stringify(result));
    check("typing after Enter stays quoted (one blockquote)", result.md.trim() === "> foo\n>\n> bar", JSON.stringify(result));
  }

  // 17. Double Enter in a quote exits it: the empty "> " line becomes a
  //     plain paragraph.
  {
    const result = await evalJs(`
      const { editor } = await window.__boot((el) => {});
      editor.setMarkdown('> foo');
      const view = editor.view; view.focus();
      await window.__wait(30);
      window.__caretEnd(view, '> foo');
      window.__key(view, 'Enter'); // -> "> foo" / "> |"
      await window.__wait(30);
      window.__key(view, 'Enter'); // empty quote line -> exit
      await window.__wait(30);
      const $h = view.state.doc.resolve(view.state.selection.head);
      view.dispatch(view.state.tr.insertText('bar', view.state.selection.head));
      await window.__wait(220);
      return { md: editor.getMarkdown(), caretParentType: $h.parent.type.name, inQuote: $h.node(1) ? $h.node(1).type.name : null };
    `);
    check("double Enter exits the quote (caret in a plain paragraph)", result.caretParentType === "paragraph" && result.md.trim() === "> foo\n\nbar", JSON.stringify(result));
  }

  // 18. Shift+Enter in a quote always continues it (never exits), even on
  //     an empty quote line.
  {
    const result = await evalJs(`
      const { editor } = await window.__boot((el) => {});
      editor.setMarkdown('> foo');
      const view = editor.view; view.focus();
      await window.__wait(30);
      window.__caretEnd(view, '> foo');
      window.__key(view, 'Enter', { shift: true }); // -> "> foo" / "> |"
      await window.__wait(30);
      window.__key(view, 'Enter', { shift: true }); // still continues (no exit)
      await window.__wait(30);
      view.dispatch(view.state.tr.insertText('bar', view.state.selection.head));
      await window.__wait(220);
      return editor.getMarkdown();
    `);
    check("Shift+Enter always continues the quote", result.trim() === "> foo\n>\n> bar", JSON.stringify(result));
  }

  // 19. Typing a TRAILING space in a list/quote line must survive the
  //     reparse — regression: the lossless guard trimmed trailing space,
  //     deleting it mid-typing so the next word stuck to the previous one.
  {
    const result = await evalJs(`
      const { editor } = await window.__boot((el) => {});
      editor.setMarkdown('- hello');
      const view = editor.view; view.focus();
      await window.__wait(30);
      const p = window.__caretEnd(view, '- hello');
      view.dispatch(view.state.tr.insertText(' ', view.state.selection.head));
      await window.__wait(200); // reparse fires on the trailing space
      const afterSpace = editor.getMarkdown();
      // then type the next word
      for (const ch of 'world') view.dispatch(view.state.tr.insertText(ch, view.state.selection.head));
      await window.__wait(200);
      return { afterSpace, afterWord: editor.getMarkdown() };
    `);
    check("trailing space in a bullet survives the reparse", result.afterSpace === "- hello ", JSON.stringify(result));
    check("word typed after a trailing space keeps the space", result.afterWord.trim() === "- hello world", JSON.stringify(result));
  }

  // 20. Caret anywhere in a multi-line blockquote reveals EVERY line's
  //     `> ` marker, not just the caret's line.
  {
    const result = await evalJs(`
      const { el, editor } = await window.__boot((el) => {});
      editor.setMarkdown('> one\\n>\\n> two\\n>\\n> three');
      const view = editor.view; view.focus();
      await window.__wait(30);
      // Put the caret in the FIRST quote line.
      window.__caretEnd(view, '> one');
      await window.__wait(30);
      // The reveal is decoration-driven: every textblock in the enclosing
      // blockquote gets the md-active node decoration (CSS then shows their
      // '> ' markers when focused). Assert the decoration reaches all three
      // lines' paragraphs — headless focus doesn't set .ProseMirror-focused,
      // so we check the decoration class, not the computed display.
      const root = el.shadowRoot;
      const quoteParas = [...root.querySelectorAll('blockquote p')];
      const active = quoteParas.filter(p => p.classList.contains('md-active'));
      const markers = [...root.querySelectorAll('.md-markup.md-block')]
        .filter(m => m.textContent === '> ');
      return { quoteParas: quoteParas.length, active: active.length, markers: markers.length };
    `);
    check("caret in a quote marks every line active (reveals all '> ')", result.quoteParas === 3 && result.active === 3, JSON.stringify(result));
  }
}

main().catch((err) => {
  console.error(err);
  server.close();
  process.exit(1);
});
