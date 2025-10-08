import { resolve } from 'path';
import { defineConfig } from 'vite';

export default defineConfig({
  build: {
    target: 'esnext',
    outDir: 'src/test-ui/public',
    emptyOutDir: false,
    rollupOptions: {
      input: resolve(__dirname, '../packages/host-web/src/service-worker.ts'),
      output: {
        entryFileNames: 'service-worker.js',
        format: 'es',
      },
    },
  },
  define: {
    'process.env': {},
    global: 'globalThis',
    TONK_SERVER_URL: JSON.stringify('http://localhost'),
  },
  resolve: {
    alias: {
      buffer: 'buffer',
      process: 'process/browser',
      util: 'util',
    },
  },
});
