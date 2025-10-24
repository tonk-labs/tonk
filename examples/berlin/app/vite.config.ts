import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import path from 'path';

// https://vite.dev/config/
export default defineConfig({
  // Set base path to match the app directory name
  base: process.env.NODE_ENV === 'development' ? '/app' : '/',

  plugins: [react(), tailwindcss()],

  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
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
    cors: {
      origin: 'http://localhost:4000', // Allow requests from host-web
      credentials: true,
    },
    hmr: {
      // WebSocket connects directly to Vite (service workers can't proxy WebSockets)
      protocol: 'ws',
      host: 'localhost',
      port: 4001,
    },
  },
});
