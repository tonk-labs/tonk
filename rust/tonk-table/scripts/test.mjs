#!/usr/bin/env node
// Test runner: esbuild-bundles every `*.test.ts` under `src-js/` into a
// temp dir (Node 20 can't strip TS types natively), then runs them with
// Node's built-in test runner. No test-framework dependency beyond
// esbuild, which the build already uses. Mirrors tonk-prose's runner,
// plus the `binary` wasm loader so tests exercise the REAL IronCalc
// engine — a test imports `../engine` and feeds the bytes to
// `initSync`, the same instantiate-from-bytes path production uses.
//
//   npm test            # run all tests
//   npm test -- <name>  # run tests whose file path contains <name>

import { build } from "esbuild";
import { fileURLToPath } from "node:url";
import { dirname, resolve, relative } from "node:path";
import { mkdirSync, rmSync, readdirSync, statSync } from "node:fs";
import { spawnSync } from "node:child_process";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = resolve(__dirname, "..");
const srcDir = resolve(root, "src-js");
const outDir = resolve(root, ".test-build");

/** Recursively collect every `*.test.ts` under `dir`. */
function findTests(dir) {
  const out = [];
  for (const name of readdirSync(dir)) {
    const full = resolve(dir, name);
    if (statSync(full).isDirectory()) out.push(...findTests(full));
    else if (name.endsWith(".test.ts")) out.push(full);
  }
  return out;
}

const filter = process.argv[2];
let tests = findTests(srcDir);
if (filter) tests = tests.filter((t) => t.includes(filter));

if (tests.length === 0) {
  console.error(filter ? `No tests match "${filter}".` : "No *.test.ts found.");
  process.exit(1);
}

rmSync(outDir, { recursive: true, force: true });
mkdirSync(outDir, { recursive: true });

// Bundle each test independently so imports resolve and TS is stripped.
const entryPoints = {};
for (const t of tests) {
  const key = relative(srcDir, t).replace(/\.test\.ts$/, ".test");
  entryPoints[key] = t;
}

await build({
  entryPoints,
  outdir: outDir,
  bundle: true,
  platform: "node",
  format: "esm",
  target: ["node20"],
  sourcemap: "inline",
  loader: { ".wasm": "binary" },
  logLevel: "warning",
});

const built = findBuilt(outDir);
const result = spawnSync(
  process.execPath,
  ["--test", ...built],
  { stdio: "inherit" },
);

rmSync(outDir, { recursive: true, force: true });
process.exit(result.status ?? 1);

function findBuilt(dir) {
  const out = [];
  for (const name of readdirSync(dir)) {
    const full = resolve(dir, name);
    if (statSync(full).isDirectory()) out.push(...findBuilt(full));
    else if (name.endsWith(".test.js")) out.push(full);
  }
  return out;
}
