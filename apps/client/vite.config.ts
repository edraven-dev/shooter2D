import { defineConfig } from 'vite';

// The lobby (cargo run -p lobby) serves the API; in production it also serves this build.
const lobby = 'http://127.0.0.1:3000';

export default defineConfig({
  server: { port: 8080, strictPort: true, proxy: { '/api': lobby } },
  preview: { port: 8080, strictPort: true, proxy: { '/api': lobby } },
  build: { target: 'es2024' },
});
