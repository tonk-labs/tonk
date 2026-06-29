#!/usr/bin/env node
// Bundle the sealed-guest Web Awesome component subset into one self-contained
// ESM (`assets/guest/wa.js`) the portal injects into the opaque-origin iframe.
//
// The guest can't use Web Awesome's normal lazy auto-loader (it does dynamic
// network `import()` of per-component chunks, dead at a null origin), so we
// esbuild a synthetic entry (`assets/guest/wa-entry.mjs`) that statically imports
// exactly the components the guest-rendered views use — importing a component
// module registers its `<wa-*>` element. Add a component to `wa-entry.mjs` and
// rerun this when a guest view starts using a new `<wa-*>` tag (a missing one is
// an inert unknown element = no visible output).
//
// Usage:  node scripts/build-wa-guest.mjs
//
// esbuild is taken from the sibling tonk-code package's node_modules (the repo's
// only JS toolchain); run `npm install` there first if it's absent.

import { build } from "../../tonk-code/node_modules/esbuild/lib/main.js";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const guest = resolve(root, "assets/guest");

await build({
  entryPoints: [resolve(guest, "wa-entry.mjs")],
  outfile: resolve(guest, "wa.js"),
  bundle: true,
  format: "esm",
  target: ["es2022"],
  minify: true,
  logLevel: "info",
});
