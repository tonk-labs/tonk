#!/usr/bin/env node
"use strict";

// Thin launcher for the tonk CLI. The real program is a native Rust
// binary shipped in a per-platform package (@tonk/cli-<platform>-<arch>).
// npm installs only the package whose `os`/`cpu` match this host, so we
// resolve that one binary and exec it. esbuild, turbo, and swc all
// distribute their native binaries with this same require.resolve pattern.
const { spawnSync } = require("node:child_process");

function resolveBinary() {
  const { platform, arch } = process;
  const pkg = `@tonk/cli-${platform}-${arch}`;
  const exe = platform === "win32" ? "tonk.exe" : "tonk";
  try {
    return require.resolve(`${pkg}/bin/${exe}`);
  } catch {
    return null;
  }
}

const bin = resolveBinary();
if (bin === null) {
  console.error(
    `@tonk/cli: no prebuilt tonk binary for ${process.platform}-${process.arch} yet.\n` +
      "Supported today: darwin-arm64 and linux-x64. More platforms coming soon.",
  );
  process.exit(1);
}

const result = spawnSync(bin, process.argv.slice(2), { stdio: "inherit" });
if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}
// spawnSync sets `status` to null when the child was killed by a signal;
// map that to a non-zero exit so callers see the failure.
process.exit(typeof result.status === "number" ? result.status : 1);
