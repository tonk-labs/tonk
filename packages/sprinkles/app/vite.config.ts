import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  server: {
    port: 4001,
    strictPort: true,
    hmr: {
      protocol: 'ws',
      host: 'localhost',
      port: 4001,
    },
  },
})
