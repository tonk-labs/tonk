import { defineConfig } from 'vite';
import { resolve } from 'path';
import react from '@vitejs/plugin-react';
import tailwindcss from "@tailwindcss/vite";
import tsconfigPaths from 'vite-tsconfig-paths';

export default defineConfig({
  plugins: [
    tailwindcss(),
    tsconfigPaths(),
    react(),
  ],
  root: resolve(__dirname, 'src/pages/test-view'),
  publicDir: resolve(__dirname, 'public'),
  build: {
    sourcemap: true,
    outDir: resolve(__dirname, 'dist_test'),
    emptyOutDir: true,
  },
  server: {
    port: 3001,
    open: true
  },
}); 