import { defineConfig } from 'tsup';

export default defineConfig({
  entry: ['src/tonk.ts'],
  format: ['esm'],
  clean: true,
  dts: true,
  minify: true,
  splitting: false,
  sourcemap: true,
  banner: {
    js: '#!/usr/bin/env node',
  },
  onSuccess: 'chmod +x dist/tonk.js',
}); 