#!/usr/bin/env node
// Bundles `src-js/index.ts` (the <tonk-prose> shell) and the heavy
// editor core into `assets/`.
//
// Output layout:
//   assets/tonk-prose.js         — shell bundle, registers the element.
//                                  Contains no ProseMirror code.
//   assets/tonk-prose-editor.js  — editor core (ProseMirror schema,
//                                  plugins, markdown round-trip),
//                                  loaded on demand via dynamic import
//                                  by the first connected element.
//   ../tonk-core/assets/library/prose-editor/core.js
//                                — the editor core again, for the prose
//                                  library component's <prose-editor>,
//                                  written as the method expression that
//                                  returns it (see below).
//
// The shell stays tiny on purpose: pages that ship the bundle but
// never render a `<tonk-prose>` element pay only for the custom
// element registration. The editor core is fetched exactly once,
// the first time an element actually connects. Code blocks inside
// the editor delegate to `<tonk-code>` (when that element is
// defined), which lazy-loads its own per-language chunks.
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
  "tonk-prose": resolve(root, "src-js/index.ts"),
  "tonk-prose-editor": resolve(root, "src-js/editor/index.ts"),
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
  // No code splitting: the shell must be ONE self-contained file. It's
  // postMessaged into sealed guests as a single string and blob-minted
  // there (tonk-portal), where a cross-file `import "./chunk-….js"`
  // can't resolve. Splitting is safe to drop because the shell and the
  // editor share no stateful module — the shell imports only the pure
  // `content`/`hlc` logic and dynamic-imports the editor entry, which
  // is emitted as its own file regardless. (The ProseMirror `schema`
  // singleton lives entirely inside the editor chunk, so there's no
  // cross-entry identity to protect, unlike a shared-schema setup.)
  splitting: false,
  external: [],
  logLevel: "info",
};

// The library component's copy of the core. <prose-editor> is branch
// data, with no URL to import a chunk from, so the core is one of its
// methods: an IIFE wrapped by the banner and footer into a function that
// evaluates the bundle and returns its exports (`createEditor`).
/** @type {import('esbuild').BuildOptions} */
const library = {
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
  const contexts = [await context(options), await context(library)];
  await Promise.all(contexts.map((ctx) => ctx.watch()));
  console.log("[tonk-prose] watching…");
} else {
  await Promise.all([build(options), build(library)]);
}
