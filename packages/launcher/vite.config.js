import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import path from 'path';
import { defineConfig } from 'vite';
// WASM Architecture:
// Dev: scripts/setup-wasm.ts copies from @tonk/core to public/
// Prod: scripts/build-runtime.ts copies to public/app/
// Both use require.resolve() to find the package regardless of hoisting
export default defineConfig({
  appType: 'mpa',
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
      '@/launcher': path.resolve(__dirname, './src/launcher'),
      '@/runtime': path.resolve(__dirname, './src/runtime'),
      '@/auth': path.resolve(__dirname, './src/auth'),
      '@/shared': path.resolve(__dirname, './src/shared'),
      '@/lib': path.resolve(__dirname, './src/lib'),
    },
  },
});
