import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';
// The extension lets Vite's native config loader resolve the module.
import { startupBudget } from './bundle-budget.ts';

const here = dirname(fileURLToPath(import.meta.url));
// The FOKS desktop frontend.
export default defineConfig({
  root: here,
  plugins: [react(), startupBudget()],
  server: {
    host: '127.0.0.1',
    port: 1421,
    strictPort: true,
  },
  build: {
    outDir: resolve(here, 'dist'),
    emptyOutDir: true,
  },
  // app-root.render.test.tsx boots src/main.tsx through `vite.ssrLoadModule`,
  // which externalises bare imports by default. `@tauri-apps/api` must be bundled
  // instead: it is ESM-only and reaches for the Tauri runtime's globals.
  ssr: {
    noExternal: ['@tauri-apps/api'],
  },
  clearScreen: false,
});
