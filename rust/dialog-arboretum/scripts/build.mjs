#!/usr/bin/env node
// Bundles `src-js/index.ts` (the <dialog-arboretum> custom element)
// into `assets/dialog-arboretum.js` — a single self-contained,
// dependency-free ES module that registers the element.
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

const watch = process.argv.includes("--watch");

/** @type {import('esbuild').BuildOptions} */
const options = {
  entryPoints: [resolve(root, "src-js/index.ts")],
  outfile: resolve(outdir, "dialog-arboretum.js"),
  bundle: true,
  format: "esm",
  target: "es2022",
  sourcemap: true,
  minify: !watch,
};

if (watch) {
  const ctx = await context(options);
  await ctx.watch();
  console.log("dialog-arboretum: watching…");
} else {
  await build(options);
  console.log("dialog-arboretum: built assets/dialog-arboretum.js");
}
