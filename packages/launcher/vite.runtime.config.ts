import { resolve } from 'path';
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';

export default defineConfig({
  base: './',
  root: 'src/runtime',
  plugins: [react(), tailwindcss()],
  build: {
    target: 'esnext',
    outDir: '../../dist-runtime', // Output to temporary dist folder
    emptyOutDir: true,
    rollupOptions: {
      input: {
        main: resolve(__dirname, 'src/runtime/index.html'),
      },
      output: {
        entryFileNames: `[name].js`,
        chunkFileNames: `[name].js`,
        assetFileNames: `[name].[ext]`,
      },
    },
  },
  define: {
    'process.env': {},
    global: 'globalThis',
    TONK_SERVE_LOCAL: JSON.stringify(
      process.env.TONK_SERVE_LOCAL === 'true'
    ),
  },
});
