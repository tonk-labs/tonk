import {defineConfig} from 'vite';
import {resolve} from 'path';
import dts from 'vite-plugin-dts';
import wasm from 'vite-plugin-wasm';

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
      external: ['react', 'zustand', '@automerge/automerge-repo', 'ws', 'chalk'],
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
  ],
  resolve: {
    extensions: ['.ts', '.tsx', '.js'],
  },
});