import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { VitePWA } from 'vite-plugin-pwa';
import wasm from 'vite-plugin-wasm'
import topLevelAwait from 'vite-plugin-top-level-await'

// https://vitejs.dev/config/
export default defineConfig(({ mode }) => {
  const isProduction = mode === 'production';
  
  return {
    plugins: [
      react(),
      wasm(), topLevelAwait(),
      isProduction && VitePWA({
        registerType: 'autoUpdate',
        devOptions: {
          enabled: true
        }
      })
    ],
    server: {
      port: 3000,
      proxy: {
        '/sync': {
          target: 'ws://localhost:7777',
          ws: true,
          changeOrigin: true
        }
      }
    },
    resolve: {
      alias: {
        // Add any path aliases here if needed
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
            automerge: ['@automerge/automerge'],
          }
        }
      }
    },
    optimizeDeps: {
      esbuildOptions: {
        target: 'esnext',
      },
    },
    esbuild: {
      target: 'esnext',
    }
  };
}); 