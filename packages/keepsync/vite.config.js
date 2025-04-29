import {defineConfig} from 'vite';
import {resolve} from 'path';
import dts from 'vite-plugin-dts';
import wasm from 'vite-plugin-wasm';
import topLevelAwait from 'vite-plugin-top-level-await';

export default defineConfig({
  build: {
    lib: {
      entry: resolve(__dirname, 'src/index.ts'),
      name: 'keepsync',
      formats: ['es'],
      fileName: 'index',
    },
    sourcemap: true,
    outDir: 'dist',
    rollupOptions: {
      external: [
        'react', 
        'zustand', 
        '@tonk/automerge-repo', 
        'ws', 
        'chalk',
        'fs',
        'path',
        'os',
        'events',
        'stream',
        'util',
        'node:events',
        'node:stream',
        'node:string_decoder',
        'node:fs',
        'node:path',
        'node:url',
      ],
      output: {
        globals: {
          react: 'React',
          zustand: 'zustand',
        },
      },
    },
    target: 'es2020',
  },
  plugins: [
    dts({
      insertTypesEntry: true,
    }),
    wasm(),
    topLevelAwait()
  ],
  resolve: {
    extensions: ['.ts', '.tsx', '.js'],
  },
  optimizeDeps: {
    include: ['@automerge/automerge-repo-storage-nodefs'],
    esbuildOptions: {
      define: {
        global: 'globalThis',
      },
    },
  },
});
