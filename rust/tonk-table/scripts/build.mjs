#!/usr/bin/env node
// Bundles the `<tonk-table>` element into `assets/` as three chunks:
//
//   assets/tonk-table-grid.js   — grid core (DOM grid + IronCalc JS
//                                 glue) plus the shell-support surface
//                                 (`grid/host.ts`). Loaded on demand,
//                                 by the element's own `connected`.
//   assets/tonk-table-engine.js — the IronCalc engine wasm, embedded as
//                                 base64 (`binary` loader) in a pure
//                                 data leaf the grid pulls in the same
//                                 way. Isolated so it only changes on an
//                                 IronCalc version bump — grid-UI edits
//                                 never rewrite the multi-megabyte file.
//
// There is no shell chunk. The `<tonk-table>` SHELL — the custom
// element around the core: what it mounts, what it watches, what it
// dispatches, what properties it exposes — is branch data, declared as
// an `element!:` in `tonk-core/assets/library/table.yaml` and resolved
// by the element registry the first time the tag is rendered. What
// stays here is what cannot be a fact: a ~4MB wasm engine and the
// TypeScript program that drives it. Both are fetched exactly once,
// the first time an element actually connects.
//
// No code splitting: each chunk must be ONE self-contained file. They
// are postMessaged into sealed guests as strings and blob-minted there
// (tonk-portal), where a cross-file `import "./chunk-….js"` can't
// resolve — the ONLY cross-chunk seams are the two runtime-variable
// dynamic imports (the branch-resident shell → grid, grid → engine),
// which esbuild leaves alone and the guest injector rewrites to blob
// URLs. Splitting is safe to drop because the chunks share no stateful
// module: the engine chunk is pure data (the wasm bytes; the
// wasm-bindgen module state lives in the grid chunk alone).
//
// The engine instantiates FROM BYTES (`init({ module_or_path })`), so
// the glue's own `new URL('wasm_bg.wasm', import.meta.url)` default-
// resolution line never executes — no `.wasm` asset is emitted or
// fetched at runtime.
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

const entryPoints = {
  "tonk-table-grid": resolve(root, "src-js/grid/index.ts"),
  "tonk-table-engine": resolve(root, "src-js/engine.ts"),
};

/** @type {import('esbuild').BuildOptions} */
const options = {
  entryPoints,
  outdir,
  bundle: true,
  format: "esm",
  target: ["es2022"],
  minify: true,
  sourcemap: true,
  splitting: false,
  // The engine wasm import (src-js/engine.ts) becomes an embedded
  // base64 string decoded to a Uint8Array at module evaluation.
  loader: { ".wasm": "binary" },
  external: [],
  logLevel: "info",
};

const watch = process.argv.includes("--watch");

if (watch) {
  const ctx = await context(options);
  await ctx.watch();
  console.log("[tonk-table] watching…");
} else {
  await build(options);
}
