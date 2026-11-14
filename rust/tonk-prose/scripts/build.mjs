#!/usr/bin/env node
// Builds the library's <prose-editor> (rust/tonk-core/assets/library/prose-editor.yaml).
//
// Branch data has no URL to import a chunk from, so the editor core
// (src-js/editor/index.ts: ProseMirror schema, plugins, markdown
// round-trip) is one of the element's methods:
//
//   library/prose-editor/core.js: an IIFE wrapped by the banner and
//                                  footer into a function that evaluates
//                                  the bundle and returns its exports
//                                  (`createEditor`, ...).
//
// Usage:
//   node scripts/build.mjs           # production build
//   node scripts/build.mjs --watch   # rebuild on file changes

import { build, context } from "esbuild";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = resolve(__dirname, "..");

/** @type {import('esbuild').BuildOptions} */
const core = {
  entryPoints: [resolve(root, "src-js/editor/index.ts")],
  outfile: resolve(root, "../tonk-core/assets/library/prose-editor/core.js"),
  bundle: true,
  format: "iife",
  globalName: "core",
  target: ["es2022"],
  minify: true,
  banner: { js: "() => {" },
  footer: { js: "return core;\n}" },
  logLevel: "info",
};

const watch = process.argv.includes("--watch");

if (watch) {
  const ctx = await context(core);
  await ctx.watch();
  console.log("[prose-editor] watching…");
} else {
  await build(core);
}
