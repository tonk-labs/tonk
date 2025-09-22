import { resolve } from "path"
import { defineConfig } from "vite"

export default defineConfig({
  base: "./",
  root: "src",

  server: {
    // Serve service worker from root for proper registration
    fs: {
      allow: ['..']
    }
  },

  build: {
    target: "esnext",
    outDir: "../dist",
    rollupOptions: {
      input: {
        main: resolve(__dirname, "src/index.html"),
      },
      output: {
        entryFileNames: `[name].js`,
        chunkFileNames: `[name].js`,
        assetFileNames: `[name].[ext]`,
      },
    },
  },
  optimizeDeps: {
    esbuildOptions: { target: "esnext" },
    include: ['@tonk/core/slim'],
  },
  define: { 
    "process.env": {},
    global: "globalThis",
  },
  resolve: {
    alias: {
      // Handle Node.js built-ins for browser
      buffer: "buffer",
      process: "process/browser", 
      util: "util",
    },
  },
})