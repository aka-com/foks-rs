import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
// The FOKS desktop frontend.
export default defineConfig({
  root: here,
  plugins: [react()],
  resolve: {
    alias: {
      // Keep absolute source aliases aligned with tsconfig.json.
      '/src': resolve(here, 'src'),
      '/kit': resolve(here, 'kit'),
    },
  },
  server: {
    host: '127.0.0.1',
    port: 1421,
    strictPort: true,
  },
  build: {
    outDir: resolve(here, 'dist'),
    emptyOutDir: true,
  },
  // The render tests boot the app through `vite.ssrLoadModule`, which
  // externalises bare imports by default. `@tauri-apps/api` must be bundled
  // instead: it is ESM-only and reaches for the Tauri runtime's globals.
  ssr: {
    noExternal: ['@tauri-apps/api'],
  },
  clearScreen: false,
});
