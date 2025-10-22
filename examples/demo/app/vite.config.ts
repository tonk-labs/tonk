import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

// https://vite.dev/config/
export default defineConfig({
  // Set base path to match the service worker proxy path
  base: '/bundling/',

  plugins: [react(), tailwindcss()],

  server: {
    port: 4001,
    strictPort: true,
    cors: {
      origin: 'http://localhost:4000', // Allow requests from host-web
      credentials: true,
    },
    hmr: {
      // WebSocket connects directly to Vite (service workers can't proxy WebSockets)
      protocol: 'ws',
      host: 'localhost',
      port: 4001,
    },
  },
})
