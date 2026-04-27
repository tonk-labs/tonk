#!/usr/bin/env node
// Bundles `src-js/index.ts` (the <tonk-code> custom element) and
// per-language entry points into `assets/`.
//
// Output layout:
//   assets/tonk-code.js             — main bundle, registers the element
//   assets/tonk-code-lang-<id>.js   — one chunk per language pack,
//                                     loaded on demand via dynamic import
//
// The main bundle is fully self-contained for the element's core
// behavior (state, view, history, default keymap) — it doesn't
// touch any language pack until a `mode` attribute requests one.
// Each language entry point exports a default `LanguageSupport`
// instance and is loaded by the element with a relative URL
// (`./tonk-code-lang-yaml.js`), which means the consumer must
// serve all bundles from the same directory.
//
// Usage:
//   node scripts/build.mjs           # production build
//   node scripts/build.mjs --watch   # rebuild on file changes

import { build, context } from "esbuild";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { mkdirSync } from "node:fs";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = resolve(__dirname, "..");
const outdir = resolve(root, "assets");
mkdirSync(outdir, { recursive: true });

// Languages to ship as separate chunks. Add to this list when
// extending mode coverage; the element resolves a `mode` attribute
// to `./tonk-code-lang-<mode>.js`, so the keys here are the public
// `mode` attribute values.
const languages = [
  { id: "yaml", entry: "src-js/lang/yaml.ts" },
];

const entryPoints = {
  "tonk-code": resolve(root, "src-js/index.ts"),
};
for (const { id, entry } of languages) {
  entryPoints[`tonk-code-lang-${id}`] = resolve(root, entry);
}

/** @type {import('esbuild').BuildOptions} */
const options = {
  entryPoints,
  outdir,
  bundle: true,
  format: "esm",
  target: ["es2022"],
  minify: true,
  sourcemap: true,
  // Splitting is required for *correctness*, not just size: the main
  // bundle and language packs share `@codemirror/state` (and friends),
  // and CodeMirror uses `instanceof` to validate `Extension` values
  // when reconfiguring. Two independent copies of `@codemirror/state`
  // — one inlined in the main bundle, one in a language chunk — make
  // those checks fail and the language pack is rejected with
  // "Unrecognized extension value in extension set".
  //
  // Splitting puts the shared CodeMirror modules in one chunk that
  // both entries import, so there's exactly one identity per type.
  // (Earlier we worried this broke style injection — that turned out
  // to be a separate issue caused by `<wa-page>`'s shadow root
  // capturing the rendered tree, since fixed by mounting the editor
  // inside our own shadow root.)
  splitting: true,
  external: [],
  logLevel: "info",
};

const watch = process.argv.includes("--watch");

if (watch) {
  const ctx = await context(options);
  await ctx.watch();
  console.log("[tonk-code] watching…");
} else {
  await build(options);
}
