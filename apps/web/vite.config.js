import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    // so the browser only ever talks to one origin; no cors dep on the rust side.
    // `changeOrigin: false` keeps the browser's own `Host` on the forwarded
    // request, which is what the server's same-origin check compares the
    // `Origin` header against - rewriting it would make every mutating
    // request look cross-origin.
    proxy: {
      '/api': { target: 'http://127.0.0.1:3001', changeOrigin: false },
      '/ws': { target: 'ws://127.0.0.1:3001', ws: true, changeOrigin: false },
    },
  },
})
