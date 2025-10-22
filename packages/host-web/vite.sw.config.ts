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
    TONK_SERVER_URL: JSON.stringify(
      process.env.NODE_ENV === 'production'
        ? 'https://relay.tonk.xyz'
        : 'http://localhost:8081'
    ),
    TONK_SERVE_LOCAL: JSON.stringify(
      process.env.NODE_ENV !== 'production'
    ),
    __SW_BUILD_TIME__: JSON.stringify(new Date().toISOString()),
    __SW_VERSION__: JSON.stringify(Date.now().toString(36)),
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
