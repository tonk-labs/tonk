import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import wasm from "vite-plugin-wasm";
import { VitePWA } from "vite-plugin-pwa";
import topLevelAwait from "vite-plugin-top-level-await";

// https://vitejs.dev/config/
export default defineConfig({
  base: process.env.VITE_BASE_PATH || "/",
  plugins: [
    wasm(),
    react(),
    topLevelAwait(),
    VitePWA({
      registerType: "autoUpdate",
      devOptions: {
        enabled: true,
      },
      manifest: {
        name: "Tonk App",
        short_name: "Tonk App",
        description: "My new Tonk App",
      },
    }),
  ],
  server: {
    port: 3000,
    proxy: {
      "/sync": {
        target: "ws://localhost:7777",
        ws: true,
        changeOrigin: true,
      },
      "/.well-known/root.json": {
        target: "http://localhost:7777",
        changeOrigin: true,
      },
      "/api": {
        target: "http://localhost:6080",
        changeOrigin: true,
        secure: false,
        configure: (proxy, _options) => {
          proxy.on('error', (err, _req, _res) => {
            console.log('proxy error', err);
          });
          proxy.on('proxyReq', (_proxyReq, req, _res) => {
            console.log('Sending Request to the Target:', req.method, req.url);
          });
          proxy.on('proxyRes', (proxyRes, req, _res) => {
            console.log('Received Response from the Target:', proxyRes.statusCode, req.url);
          });
        },
      },
    },
  },
  build: {
    sourcemap: true,
    outDir: "dist",
    assetsDir: "assets",
    rollupOptions: {
      // Improves chunking to address the large file size warning
      output: {
        manualChunks: {
          vendor: ["react", "react-dom", "react-router-dom"],
          automerge: ["@automerge/automerge"],
        },
      },
    },
  },
  optimizeDeps: {
    esbuildOptions: {
      target: "esnext",
    },
  },
  esbuild: {
    target: "esnext",
  },
});
