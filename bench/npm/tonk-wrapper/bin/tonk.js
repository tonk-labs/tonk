#!/usr/bin/env node
// Bench-local wrapper: exec the vendored release binary. The real
// published package (PR 2) resolves a platform binary package instead.
const { spawnSync } = require("node:child_process");
const path = require("node:path");
const bin = path.join(__dirname, "..", "vendor", "tonk");
const r = spawnSync(bin, process.argv.slice(2), { stdio: "inherit" });
if (r.error) { console.error(r.error.message); process.exit(1); }
process.exit(r.status === null ? 1 : r.status);
