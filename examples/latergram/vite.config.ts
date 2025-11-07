import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Conditionally import dev-only plugins
async function loadDevPlugins() {
  const wasm = (await import('vite-plugin-wasm')).default;
  const topLevelAwait = (await import('vite-plugin-top-level-await')).default;

  return [wasm(), topLevelAwait()];
}

// https://vitejs.dev/config/
export default defineConfig(async ({ mode }) => {
  const isDev = mode === 'development';

  // Only load these plugins in development
  const plugins = [react()];

  if (isDev) {
    const devPlugins = await loadDevPlugins();
    plugins.push(...devPlugins);
  }

  return {
    base: process.env.VITE_BASE_PATH || '/',
    plugins,
    server: {
      proxy: {
        '/api':
          process.env.MANIFEST_URL ||
          'http://ec2-16-16-146-55.eu-north-1.compute.amazonaws.com:8080',
      },
    },
    build: {
      sourcemap: isDev,
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
    worker: isDev
      ? {
          format: 'es',
        }
      : undefined,
    optimizeDeps: {
      esbuildOptions: {
        target: isDev ? 'esnext' : 'es2020',
      },
      exclude: isDev ? [] : ['vite-plugin-wasm', 'vite-plugin-top-level-await'],
    },
    esbuild: {
      target: isDev ? 'esnext' : 'es2020',
    },
    assetsInclude: ['**/*.md'],
  };
});
