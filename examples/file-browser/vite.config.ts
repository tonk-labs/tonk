import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';
import { VitePWA } from 'vite-plugin-pwa';
import topLevelAwait from 'vite-plugin-top-level-await';
import wasm from 'vite-plugin-wasm';

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
    }),
  ],
  server: {
    port: 3000,
    proxy: {
      '/sync': {
        target: 'ws://localhost:7777',
        ws: true,
        changeOrigin: true,
      },
      '/.well-known/root.json': {
        target: 'http://localhost:7777',
        changeOrigin: true,
      },
      '/api': {
        target: 'http://localhost:6080',
        changeOrigin: true,
        secure: false,
      },
    },
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
          automerge: ['@automerge/automerge'],
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
});
