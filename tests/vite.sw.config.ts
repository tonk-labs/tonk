import { resolve } from 'path';
import { defineConfig } from 'vite';

export default defineConfig({
  build: {
    target: 'esnext',
    outDir: 'src/test-ui/public',
    emptyOutDir: false,
    rollupOptions: {
      input: resolve(__dirname, '../packages/launcher/src/launcher/sw/index.ts'),
      output: {
        entryFileNames: 'service-worker.js',
        format: 'es',
      },
    },
  },
  optimizeDeps: {
    esbuildOptions: { target: 'esnext' },
    include: ['@tonk/core/slim'],
  },
  define: {
    'process.env': {},
    global: 'globalThis',
    TONK_SERVER_URL: JSON.stringify(
      process.env.NODE_ENV === 'production'
        ? 'https://relay.tonk.xyz'
        : 'http://localhost:8081'
    ),
    TONK_SERVE_LOCAL: JSON.stringify(process.env.NODE_ENV !== 'production'),
    __SW_BUILD_TIME__: JSON.stringify(new Date().toISOString()),
    __SW_VERSION__: JSON.stringify(Date.now().toString(36)),
  },
  resolve: {
    alias: {
      buffer: 'buffer',
      process: 'process/browser',
      util: 'util',
      // Resolve workspace packages (order matters - more specific first)
      '@tonk/core/slim': resolve(__dirname, '../packages/core-js/dist/index-slim.js'),
      '@tonk/core': resolve(__dirname, '../packages/core-js'),
    },
  },
});
