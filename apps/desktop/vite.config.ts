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
    // A feature's index.ts only re-exports. Declared free of side effects, it
    // lets Rolldown resolve each import through it to the defining module, so
    // a deferred screen's chunk loads only the modules it uses. Otherwise
    // every module a barrel re-exports joins one chunk group, and a screen
    // that uses one export also loads the chunks holding the others.
    rolldownOptions: {
      treeshake: {
        moduleSideEffects: [
          { test: /\/src\/features\/[^/]+\/index\.ts$/, sideEffects: false },
        ],
      },
    },
  },
  // app-root.render.test.tsx boots src/main.tsx through `vite.ssrLoadModule`,
  // which externalises bare imports by default. `@tauri-apps/api` must be bundled
  // instead: it is ESM-only and reaches for the Tauri runtime's globals.
  ssr: {
    noExternal: ['@tauri-apps/api'],
  },
  clearScreen: false,
});
