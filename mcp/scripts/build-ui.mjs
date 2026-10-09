import { build } from 'esbuild';
import { fileURLToPath } from 'node:url';
await build({ entryPoints: [fileURLToPath(new URL('../ui/notebook-entry.mjs', import.meta.url))],
  outfile: fileURLToPath(new URL('../ui/notebook-renderer.js', import.meta.url)),
  bundle: true, format: 'iife', globalName: 'TonkProse', minify: true,
  legalComments: 'inline', target: 'es2022' });
