import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, '..');

// The FOKS desktop frontend. A sibling of `ui/vite.config.ts`, not a variant
// of it: the two apps share the toolchain and `ui/kit`, and nothing else.
export default defineConfig({
  root: here,
  plugins: [react()],
  resolve: {
    alias: {
      // `/src/*` is how the app imports itself, matching `ui/`'s convention
      // and the `paths` entry in tsconfig.json.
      '/src': resolve(here, 'src'),
      // `/kit/*` is the shared kit in `ui/kit` — the only thing reached
      // outside this directory. See ui/kit/README.md.
      '/kit': resolve(repo, 'ui/kit'),
    },
  },
  server: {
    host: '127.0.0.1',
    // 1421, one past AKA's 1420, so both dev servers can run at once.
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
