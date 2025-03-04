import {defineConfig} from 'tsup';

export default defineConfig({
  entry: ['src/index.ts'],
  format: ['esm', 'cjs'],
  dts: {
    entry: 'src/index.ts',
    compilerOptions: {
      moduleResolution: 'node',
      composite: false,
    },
  },
  splitting: false,
  clean: true,
  treeshake: true,
  sourcemap: true,
  outDir: 'dist',
  tsconfig: 'tsconfig.build.json',
  noExternal: [],
  external: ['fs', 'fs/promises', 'path', 'url'],
  platform: 'node',
  target: 'node18',
  banner: {
    js: '#!/usr/bin/env node',
  },
  shims: true,
});
