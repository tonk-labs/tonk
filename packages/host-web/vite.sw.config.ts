import { resolve } from 'path';
import { defineConfig } from 'vite';

export default defineConfig({
  build: {
    target: 'esnext',
    outDir: 'dist-sw',
    emptyOutDir: true,
    rollupOptions: {
      input: resolve(__dirname, './src/service-worker.ts'),
      output: {
        entryFileNames: 'service-worker-bundled.js',
        format: 'es',
      },
    },
  },
  define: {
    'process.env': {},
    global: 'globalThis',
  },
  resolve: {
    alias: {
      // Handle Node.js built-ins for browser
      buffer: 'buffer',
      process: 'process/browser',
      util: 'util',
    },
  },
});
