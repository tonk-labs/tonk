import { resolve } from 'path';
import { defineConfig } from 'vite';
import preact from '@preact/preset-vite';
import tailwindcss from '@tailwindcss/vite';

export default defineConfig({
  base: './',
  root: 'src',

  plugins: [preact(), tailwindcss()],

  build: {
    target: 'esnext',
    outDir: '../dist',
    rollupOptions: {
      input: {
        main: resolve(__dirname, 'src', 'index.html'),
        '404': resolve(__dirname, 'src', '404.html'),
      },
      output: {
        entryFileNames: `[name].js`,
        chunkFileNames: `[name].js`,
        assetFileNames: `[name].[ext]`,
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
    'window.TONK_SERVER_URL': JSON.stringify(
      process.env.NODE_ENV === 'production'
        ? 'https://relay.tonk.xyz'
        : 'http://localhost:8081'
    ),
    'window.TONK_SERVE_LOCAL': JSON.stringify(
      process.env.NODE_ENV !== 'production'
    ),
  },
  resolve: {
    alias: {
      // Handle Node.js built-ins for browser
      buffer: 'buffer',
      process: 'process/browser',
      util: 'util',
    },
  },

  server: {
    allowedHosts: ['ensemble.tonk.xyz'],
  },
});
