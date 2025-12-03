import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import path from 'path';

// https://vite.dev/config/
export default defineConfig({
  // Set base path to relative so it works under /runtime/{id}/{app}/
  base: './',

  plugins: [react(), tailwindcss()],

  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
      '@components': path.resolve(__dirname, './src/components'),
      '@features': path.resolve(__dirname, './src/features'),
      '@lib': path.resolve(__dirname, './src/lib'),
    },
  },

  optimizeDeps: {
    include: ['immer', 'zustand'],
  },

  build: {
    commonjsOptions: {
      include: [/immer/, /node_modules/],
    },
  },

  server: {
    port: 4001,
    strictPort: true,
    cors: true, // Allow all origins in development (launcher on 5173, etc.)
    hmr: {
      // WebSocket connects directly to Vite (service workers can't proxy WebSockets)
      protocol: 'ws',
      host: 'localhost',
      port: 4001,
    },
  },
});
