import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import wasm from 'vite-plugin-wasm';
import { VitePWA } from 'vite-plugin-pwa';
import topLevelAwait from 'vite-plugin-top-level-await';

// https://vitejs.dev/config/
export default defineConfig({
  base: process.env.VITE_BASE_PATH || '/',
  plugins: [
    wasm(),
    react(),
    topLevelAwait(),
    VitePWA({
      registerType: 'autoUpdate',
      devOptions: {
        enabled: true,
        
      },
      manifest: {
        name: 'Tonk App',
        short_name: 'Tonk App',
        description: 'My new Tonk App',
      },
      workbox: {
        additionalManifestEntries: [
          { url: '/ts-compiler-sw.js', revision: null },
        ],
        disableDevLogs: true
      },
    }),
  ],
  server: {
    proxy: {
      '/api': process.env.MANIFEST_URL || 'http://localhost:8081'
    }
  },
  build: {
    sourcemap: true,
    outDir: 'dist',
    assetsDir: 'assets',
    rollupOptions: {
      // Improves chunking to address the large file size warning
      output: {
        manualChunks: {
          vendor: ['react', 'react-dom', 'react-router-dom'],
        },
      },
    },
  },
  optimizeDeps: {
    esbuildOptions: {
      target: 'esnext',
    },
  },
  esbuild: {
    target: 'esnext',
  },
  assetsInclude: ['**/*.md'],
});
