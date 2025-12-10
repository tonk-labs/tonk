// vite.config.ts

import react from 'file:///Users/jackdouglas/tonk/tonk/examples/file-browser/node_modules/.pnpm/@vitejs+plugin-react@4.4.1_vite@5.4.19_@types+node@22.13.14_terser@5.39.0_/node_modules/@vitejs/plugin-react/dist/index.mjs';
import { defineConfig } from 'file:///Users/jackdouglas/tonk/tonk/examples/file-browser/node_modules/.pnpm/vite@5.4.19_@types+node@22.13.14_terser@5.39.0/node_modules/vite/dist/node/index.js';
import { VitePWA } from 'file:///Users/jackdouglas/tonk/tonk/examples/file-browser/node_modules/.pnpm/vite-plugin-pwa@0.17.5_vite@5.4.19_@types+node@22.13.14_terser@5.39.0__workbox-build@7.3.0_@t_m2pxojcojnfxpgaxz32wipnpbi/node_modules/vite-plugin-pwa/dist/index.js';
import topLevelAwait from 'file:///Users/jackdouglas/tonk/tonk/examples/file-browser/node_modules/.pnpm/vite-plugin-top-level-await@1.5.0_rollup@2.79.2_vite@5.4.19_@types+node@22.13.14_terser@5.39.0_/node_modules/vite-plugin-top-level-await/exports/import.mjs';
import wasm from 'file:///Users/jackdouglas/tonk/tonk/examples/file-browser/node_modules/.pnpm/vite-plugin-wasm@3.4.1_vite@5.4.19_@types+node@22.13.14_terser@5.39.0_/node_modules/vite-plugin-wasm/exports/import.mjs';

var vite_config_default = defineConfig({
  base: process.env.VITE_BASE_PATH || '/',
  plugins: [
    wasm(),
    react(),
    topLevelAwait(),
    VitePWA({
      registerType: 'autoUpdate',
      devOptions: {
        enabled: true,
      },
      manifest: {
        name: 'Tonk App',
        short_name: 'Tonk App',
        description: 'My new Tonk App',
      },
    }),
  ],
  server: {
    port: 3e3,
    proxy: {
      '/sync': {
        target: 'ws://localhost:7777',
        ws: true,
        changeOrigin: true,
      },
      '/.well-known/root.json': {
        target: 'http://localhost:7777',
        changeOrigin: true,
      },
      '/api': {
        target: 'http://localhost:6080',
        changeOrigin: true,
        secure: false,
      },
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
          automerge: ['@automerge/automerge'],
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
export { vite_config_default as default };
//# sourceMappingURL=data:application/json;base64,ewogICJ2ZXJzaW9uIjogMywKICAic291cmNlcyI6IFsidml0ZS5jb25maWcudHMiXSwKICAic291cmNlc0NvbnRlbnQiOiBbImNvbnN0IF9fdml0ZV9pbmplY3RlZF9vcmlnaW5hbF9kaXJuYW1lID0gXCIvVXNlcnMvamFja2RvdWdsYXMvdG9uay90b25rL2V4YW1wbGVzL2ZpbGUtYnJvd3NlclwiO2NvbnN0IF9fdml0ZV9pbmplY3RlZF9vcmlnaW5hbF9maWxlbmFtZSA9IFwiL1VzZXJzL2phY2tkb3VnbGFzL3RvbmsvdG9uay9leGFtcGxlcy9maWxlLWJyb3dzZXIvdml0ZS5jb25maWcudHNcIjtjb25zdCBfX3ZpdGVfaW5qZWN0ZWRfb3JpZ2luYWxfaW1wb3J0X21ldGFfdXJsID0gXCJmaWxlOi8vL1VzZXJzL2phY2tkb3VnbGFzL3RvbmsvdG9uay9leGFtcGxlcy9maWxlLWJyb3dzZXIvdml0ZS5jb25maWcudHNcIjtpbXBvcnQgeyBkZWZpbmVDb25maWcgfSBmcm9tIFwidml0ZVwiO1xuaW1wb3J0IHJlYWN0IGZyb20gXCJAdml0ZWpzL3BsdWdpbi1yZWFjdFwiO1xuaW1wb3J0IHdhc20gZnJvbSBcInZpdGUtcGx1Z2luLXdhc21cIjtcbmltcG9ydCB7IFZpdGVQV0EgfSBmcm9tIFwidml0ZS1wbHVnaW4tcHdhXCI7XG5pbXBvcnQgdG9wTGV2ZWxBd2FpdCBmcm9tIFwidml0ZS1wbHVnaW4tdG9wLWxldmVsLWF3YWl0XCI7XG5cbi8vIGh0dHBzOi8vdml0ZWpzLmRldi9jb25maWcvXG5leHBvcnQgZGVmYXVsdCBkZWZpbmVDb25maWcoe1xuICBiYXNlOiBwcm9jZXNzLmVudi5WSVRFX0JBU0VfUEFUSCB8fCBcIi9cIixcbiAgcGx1Z2luczogW1xuICAgIHdhc20oKSxcbiAgICByZWFjdCgpLFxuICAgIHRvcExldmVsQXdhaXQoKSxcbiAgICBWaXRlUFdBKHtcbiAgICAgIHJlZ2lzdGVyVHlwZTogXCJhdXRvVXBkYXRlXCIsXG4gICAgICBkZXZPcHRpb25zOiB7XG4gICAgICAgIGVuYWJsZWQ6IHRydWUsXG4gICAgICB9LFxuICAgICAgbWFuaWZlc3Q6IHtcbiAgICAgICAgbmFtZTogXCJUb25rIEFwcFwiLFxuICAgICAgICBzaG9ydF9uYW1lOiBcIlRvbmsgQXBwXCIsXG4gICAgICAgIGRlc2NyaXB0aW9uOiBcIk15IG5ldyBUb25rIEFwcFwiLFxuICAgICAgfSxcbiAgICB9KSxcbiAgXSxcbiAgc2VydmVyOiB7XG4gICAgcG9ydDogMzAwMCxcbiAgICBwcm94eToge1xuICAgICAgXCIvc3luY1wiOiB7XG4gICAgICAgIHRhcmdldDogXCJ3czovL2xvY2FsaG9zdDo3Nzc3XCIsXG4gICAgICAgIHdzOiB0cnVlLFxuICAgICAgICBjaGFuZ2VPcmlnaW46IHRydWUsXG4gICAgICB9LFxuICAgICAgXCIvLndlbGwta25vd24vcm9vdC5qc29uXCI6IHtcbiAgICAgICAgdGFyZ2V0OiBcImh0dHA6Ly9sb2NhbGhvc3Q6Nzc3N1wiLFxuICAgICAgICBjaGFuZ2VPcmlnaW46IHRydWUsXG4gICAgICB9LFxuICAgICAgXCIvYXBpXCI6IHtcbiAgICAgICAgdGFyZ2V0OiBcImh0dHA6Ly9sb2NhbGhvc3Q6NjA4MFwiLFxuICAgICAgICBjaGFuZ2VPcmlnaW46IHRydWUsXG4gICAgICAgIHNlY3VyZTogZmFsc2UsXG4gICAgICB9LFxuICAgIH0sXG4gIH0sXG4gIGJ1aWxkOiB7XG4gICAgc291cmNlbWFwOiB0cnVlLFxuICAgIG91dERpcjogXCJkaXN0XCIsXG4gICAgYXNzZXRzRGlyOiBcImFzc2V0c1wiLFxuICAgIHJvbGx1cE9wdGlvbnM6IHtcbiAgICAgIC8vIEltcHJvdmVzIGNodW5raW5nIHRvIGFkZHJlc3MgdGhlIGxhcmdlIGZpbGUgc2l6ZSB3YXJuaW5nXG4gICAgICBvdXRwdXQ6IHtcbiAgICAgICAgbWFudWFsQ2h1bmtzOiB7XG4gICAgICAgICAgdmVuZG9yOiBbXCJyZWFjdFwiLCBcInJlYWN0LWRvbVwiLCBcInJlYWN0LXJvdXRlci1kb21cIl0sXG4gICAgICAgICAgYXV0b21lcmdlOiBbXCJAYXV0b21lcmdlL2F1dG9tZXJnZVwiXSxcbiAgICAgICAgfSxcbiAgICAgIH0sXG4gICAgfSxcbiAgfSxcbiAgb3B0aW1pemVEZXBzOiB7XG4gICAgZXNidWlsZE9wdGlvbnM6IHtcbiAgICAgIHRhcmdldDogXCJlc25leHRcIixcbiAgICB9LFxuICB9LFxuICBlc2J1aWxkOiB7XG4gICAgdGFyZ2V0OiBcImVzbmV4dFwiLFxuICB9LFxufSk7XG4iXSwKICAibWFwcGluZ3MiOiAiO0FBQXdVLFNBQVMsb0JBQW9CO0FBQ3JXLE9BQU8sV0FBVztBQUNsQixPQUFPLFVBQVU7QUFDakIsU0FBUyxlQUFlO0FBQ3hCLE9BQU8sbUJBQW1CO0FBRzFCLElBQU8sc0JBQVEsYUFBYTtBQUFBLEVBQzFCLE1BQU0sUUFBUSxJQUFJLGtCQUFrQjtBQUFBLEVBQ3BDLFNBQVM7QUFBQSxJQUNQLEtBQUs7QUFBQSxJQUNMLE1BQU07QUFBQSxJQUNOLGNBQWM7QUFBQSxJQUNkLFFBQVE7QUFBQSxNQUNOLGNBQWM7QUFBQSxNQUNkLFlBQVk7QUFBQSxRQUNWLFNBQVM7QUFBQSxNQUNYO0FBQUEsTUFDQSxVQUFVO0FBQUEsUUFDUixNQUFNO0FBQUEsUUFDTixZQUFZO0FBQUEsUUFDWixhQUFhO0FBQUEsTUFDZjtBQUFBLElBQ0YsQ0FBQztBQUFBLEVBQ0g7QUFBQSxFQUNBLFFBQVE7QUFBQSxJQUNOLE1BQU07QUFBQSxJQUNOLE9BQU87QUFBQSxNQUNMLFNBQVM7QUFBQSxRQUNQLFFBQVE7QUFBQSxRQUNSLElBQUk7QUFBQSxRQUNKLGNBQWM7QUFBQSxNQUNoQjtBQUFBLE1BQ0EsMEJBQTBCO0FBQUEsUUFDeEIsUUFBUTtBQUFBLFFBQ1IsY0FBYztBQUFBLE1BQ2hCO0FBQUEsTUFDQSxRQUFRO0FBQUEsUUFDTixRQUFRO0FBQUEsUUFDUixjQUFjO0FBQUEsUUFDZCxRQUFRO0FBQUEsTUFDVjtBQUFBLElBQ0Y7QUFBQSxFQUNGO0FBQUEsRUFDQSxPQUFPO0FBQUEsSUFDTCxXQUFXO0FBQUEsSUFDWCxRQUFRO0FBQUEsSUFDUixXQUFXO0FBQUEsSUFDWCxlQUFlO0FBQUE7QUFBQSxNQUViLFFBQVE7QUFBQSxRQUNOLGNBQWM7QUFBQSxVQUNaLFFBQVEsQ0FBQyxTQUFTLGFBQWEsa0JBQWtCO0FBQUEsVUFDakQsV0FBVyxDQUFDLHNCQUFzQjtBQUFBLFFBQ3BDO0FBQUEsTUFDRjtBQUFBLElBQ0Y7QUFBQSxFQUNGO0FBQUEsRUFDQSxjQUFjO0FBQUEsSUFDWixnQkFBZ0I7QUFBQSxNQUNkLFFBQVE7QUFBQSxJQUNWO0FBQUEsRUFDRjtBQUFBLEVBQ0EsU0FBUztBQUFBLElBQ1AsUUFBUTtBQUFBLEVBQ1Y7QUFDRixDQUFDOyIsCiAgIm5hbWVzIjogW10KfQo=
