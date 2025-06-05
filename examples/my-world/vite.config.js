import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import wasm from "vite-plugin-wasm";
import { VitePWA } from "vite-plugin-pwa";
import topLevelAwait from "vite-plugin-top-level-await";

// https://vitejs.dev/config/
export default defineConfig({
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
        name: "My World",
        short_name: "My World",
        description: "Share cool places with friends",
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
