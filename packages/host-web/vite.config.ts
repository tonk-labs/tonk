import { resolve } from 'path';
import { defineConfig } from 'vite';

export default defineConfig({
  base: './',
  root: 'src',

  server: {
    // Serve service worker from root for proper registration
    fs: {
      allow: ['..'],
    },
    proxy: {
      // Proxy app requests to React dev server when in development
      '/dev-proxy': {
        target: 'http://localhost:3000',
        changeOrigin: true,
        rewrite: path => path.replace(/^\/dev-proxy/, ''),
        configure: proxy => {
          proxy.on('error', (err, req, res) => {
            console.log('Dev proxy error:', err.message);
          });
        },
      },
    },
  },

  build: {
    target: 'esnext',
    outDir: '../dist',
    rollupOptions: {
      input: {
        main: resolve(__dirname, 'src/index.html'),
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
    // Pass development mode flag to the service worker
    __DEV_MODE__: JSON.stringify(process.env.NODE_ENV === 'development'),
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
