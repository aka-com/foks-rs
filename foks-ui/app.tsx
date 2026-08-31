/**
 * The FOKS desktop frontend's entry point.
 *
 * One window (`main`), so there is no chrome to choose from the hash the way
 * `ui/app.tsx` does. It mounts the shell and gets out of the way; everything
 * the shell knows arrives through `src/bridge.ts`.
 */

import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from '/src/app-root';

const root = document.getElementById('root');
if (!root) throw new Error('#root is missing from index.html');

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
