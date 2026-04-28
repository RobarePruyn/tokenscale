import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

// During development the Rust API server runs on a separate port. The proxy
// keeps the frontend's relative `/api/*` URLs working in both dev and embedded
// production builds — no environment-specific URL plumbing required.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    proxy: {
      '/api': {
        target: 'http://127.0.0.1:8787',
        changeOrigin: false,
      },
    },
  },
  build: {
    // The Rust binary embeds frontend/dist/ via rust-embed. Keeping the build
    // deterministic (no per-run hashes in unhashed asset names) helps when
    // diffing release artefacts.
    outDir: 'dist',
    emptyOutDir: true,
  },
})
