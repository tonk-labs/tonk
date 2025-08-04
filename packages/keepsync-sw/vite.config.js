import { defineConfig } from 'vite';
import { resolve } from 'path';
import dts from 'vite-plugin-dts';

export default defineConfig({
  build: {
    lib: {
      entry: {
        index: resolve(__dirname, 'src/index.ts'),
      },
      name: 'keepsync',
      formats: ['es'],
      fileName: (_, entryName) => `${entryName}.js`,
    },
    sourcemap: true,
    outDir: 'dist',
    rollupOptions: {
      external: [
        'react',
        'zustand',
        '@automerge/automerge-repo',
        '@automerge/automerge',
        'ws',
        'chalk'
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
  ],
  resolve: {
    extensions: ['.ts', '.tsx', '.js'],
  },
});
