import { resolve } from 'path';
import { defineConfig } from 'vite';

// Note: WASM is loaded at runtime by service worker from /tonk_core_bg.wasm
// Dev: served by vite-plugin-static-copy in vite.config.ts
// Prod: copied by build-host-web.ts using require.resolve()

export default defineConfig({
  build: {
    target: 'esnext',
    // We will output to a dist folder inside runtime or a temp folder, handled by build script
    outDir: 'dist-sw',
    emptyOutDir: true,
    rollupOptions: {
      input: resolve(__dirname, 'src/launcher/sw/index.ts'),
      output: {
        entryFileNames: 'service-worker-bundled.js',
        format: 'es',
        inlineDynamicImports: true,
        // Keep WASM filename stable without hash
        assetFileNames: (assetInfo) => {
          if (assetInfo.name?.endsWith('.wasm')) {
            return 'tonk_core_bg.wasm';
          }
          return 'assets/[name]-[hash][extname]';
        },
      },
    },
  },
  define: {
    'process.env': {},
    global: 'globalThis',
    window: 'self',
    TONK_SERVER_URL: JSON.stringify(
      process.env.TONK_SERVER_URL ??
        (process.env.NODE_ENV === 'production'
          ? 'https://relay.tonk.xyz'
          : 'http://localhost:8081')
    ),
    TONK_SERVE_LOCAL: JSON.stringify(
      process.env.TONK_SERVE_LOCAL === 'true'
    ),
    __SW_BUILD_TIME__: JSON.stringify(new Date().toISOString()),
    __SW_VERSION__: JSON.stringify(Date.now().toString(36)),
  },
});
