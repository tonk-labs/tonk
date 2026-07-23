#!/usr/bin/env node
// Bundles the `<tonk-table>` element into `assets/` as three chunks:
//
//   assets/tonk-table.js        — shell bundle, registers the element.
//                                 Contains no engine or grid code.
//   assets/tonk-table-grid.js   — grid core (DOM grid + IronCalc JS
//                                 glue), loaded on demand via dynamic
//                                 import by the first connected element.
//   assets/tonk-table-engine.js — the IronCalc engine wasm, embedded as
//                                 base64 (`binary` loader) in a pure
//                                 data leaf the grid pulls in the same
//                                 way. Isolated so it only changes on an
//                                 IronCalc version bump — grid-UI edits
//                                 never rewrite the multi-megabyte file.
//
// The shell stays tiny on purpose: pages that ship the bundle but never
// render a `<tonk-table>` element pay only for the custom element
// registration. The grid core (and then the engine bytes) are fetched
// exactly once, the first time an element actually connects.
//
// No code splitting: each chunk must be ONE self-contained file. They
// are postMessaged into sealed guests as strings and blob-minted there
// (tonk-portal), where a cross-file `import "./chunk-….js"` can't
// resolve — the ONLY cross-chunk seams are the two runtime-variable
// dynamic imports (shell → grid, grid → engine), which esbuild leaves
// alone and the guest injector rewrites to blob URLs. Splitting is safe
// to drop because the chunks share no stateful module: the shell
// imports only the pure hlc/content/b64 logic, and the engine chunk is
// pure data (the wasm bytes; the wasm-bindgen module state lives in the
// grid chunk alone).
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
  "tonk-table": resolve(root, "src-js/index.ts"),
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
