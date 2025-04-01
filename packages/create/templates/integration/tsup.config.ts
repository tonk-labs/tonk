import { defineConfig } from "tsup";

export default defineConfig([
  {
    // Library build configuration
    entry: ["src/index.ts"],
    format: ["esm", "cjs"],
    dts: true,
    splitting: false,
    sourcemap: true,
    clean: true,
    treeshake: true,
    minify: true,
    target: ["es2020", "node18"],
    outDir: "dist",
  },
  {
    // CLI build configuration
    entry: ["src/bin.ts"],
    format: ["esm"],
    banner: {
      js: "#!/usr/bin/env node",
    },
    platform: "node",
    splitting: false,
    sourcemap: true,
    treeshake: true,
    minify: true,
    target: ["es2020", "node18"],
    outDir: "dist",
  },
]);
