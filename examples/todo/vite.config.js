import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { resolve } from "path";
import wasm from "vite-plugin-wasm";

// https://vitejs.dev/config/
export default defineConfig(({ mode }) => {
  const isProduction = mode === "production";

  return {
    plugins: [wasm(), react()],
    build: {
      target: "esnext",
      outDir: "dist",
      sourcemap: !isProduction,
      chunkSizeWarningLimit: 512, // in kB
      rollupOptions: {
        input: {
          main: resolve(__dirname, "index.html"),
        },
        output: {
          manualChunks: undefined,
        },
      },
    },
    worker: {
      format: "es",
      plugins: () => [wasm()],
    },
    define: {
      "process.env.NODE_ENV": JSON.stringify(
        isProduction ? "production" : "development",
      ),
    },
  };
});
