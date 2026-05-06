#!/usr/bin/env node
// Bundles `src-js/index.tsx` (the <tonk-portals> custom element +
// React app) into `assets/tonk-portals.js`.
//
// Output:
//   assets/tonk-portals.js    — main bundle, registers the element
//   assets/tonk-portals.css   — extracted CSS (loaded by the element
//                               at registration time as a constructed
//                               stylesheet so it survives shadow-root
//                               style isolation, if we end up using
//                               one — for now styles are scoped to a
//                               .tonk-portals-root container).
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

/** @type {import('esbuild').BuildOptions} */
const options = {
  entryPoints: {
    "tonk-portals": resolve(root, "src-js/index.tsx"),
  },
  outdir,
  bundle: true,
  format: "esm",
  target: ["es2022"],
  minify: true,
  sourcemap: true,
  jsx: "automatic",
  loader: { ".css": "text" },
  logLevel: "info",
};

const watch = process.argv.includes("--watch");

if (watch) {
  const ctx = await context(options);
  await ctx.watch();
  console.log("[tonk-portals] watching…");
} else {
  await build(options);
}
