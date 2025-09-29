import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import path from 'path';
// import wasm from 'vite-plugin-wasm';
// import topLevelAwait from 'vite-plugin-top-level-await';

// Auto-detect project name from current directory
const getProjectBasePath = () => {
  if (process.env.VITE_BASE_PATH) {
    return process.env.VITE_BASE_PATH;
  }

  // Get the current directory name (project folder name)
  const projectName = path.basename(process.cwd());
  return `/${projectName}/`;
};

// https://vitejs.dev/config/
export default defineConfig({
  base: getProjectBasePath(),
  plugins: [react()],
  define: {
    // Make the base path available to the app at runtime
    'import.meta.env.VITE_BASE_PATH': JSON.stringify(getProjectBasePath()),
  },
  server: {
    port: 3000,
    cors: true,
    headers: {
      'Access-Control-Allow-Origin': '*',
      'Access-Control-Allow-Methods': 'GET, POST, PUT, DELETE, OPTIONS',
      'Access-Control-Allow-Headers':
        'Origin, X-Requested-With, Content-Type, Accept, Authorization',
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
