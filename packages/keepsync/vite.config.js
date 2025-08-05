import { defineConfig } from 'vite';
import { resolve } from 'path';
import dts from 'vite-plugin-dts';
import wasm from 'vite-plugin-wasm';
import topLevelAwait from 'vite-plugin-top-level-await';

export default defineConfig(() => {
  const isSlim = process.env.BUILD_SLIM === 'true';

  return {
    build: {
      lib: {
        entry: isSlim
          ? resolve(__dirname, 'src/slim.ts')
          : resolve(__dirname, 'src/index.ts'),
        name: 'keepsync',
        formats: ['es'],
        fileName: isSlim ? 'slim' : 'index',
      },
      sourcemap: true,
      outDir: isSlim ? 'dist-slim' : 'dist',
      rollupOptions: {
        external: ['react', 'zustand', '@automerge/automerge-repo', '@automerge/automerge', 'ws', 'chalk'],
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
      ...(isSlim ? [] : [wasm(), topLevelAwait()]),
    ],
    resolve: {
      extensions: ['.ts', '.tsx', '.js'],
    }
  };
});
