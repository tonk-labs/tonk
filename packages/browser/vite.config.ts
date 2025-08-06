import { resolve } from "path"
import { defineConfig } from "vite"
import wasm from "vite-plugin-wasm"

export default defineConfig({
  base: "./",
  root: "src",

  build: {
    outDir: "../dist",
    target: "esnext",
    rollupOptions: {
      input: {
        main: resolve(__dirname, "src/index.html"),
        "service-worker": resolve(__dirname, "src/service-worker.js"),
      },
      output: {
        entryFileNames: `[name].js`,
        chunkFileNames: `[name].js`,
        assetFileNames: `[name].[ext]`,
      },
      external: [
        // Keep ESM.sh imports external
        /^https:\/\/esm\.sh\//
      ]
    },
  },
  optimizeDeps: {
    esbuildOptions: { target: "esnext" },
    exclude: ['@automerge/automerge']
  },
  plugins: [
    wasm(),
  ],
  define: { "process.env": {} },
})
