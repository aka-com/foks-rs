/**
 * The FOKS desktop frontend's entry point.
 *
 * Mounts the main application shell with state and commands provided
 * through `src/bridge.ts`.
 */

import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from '/src/app-root';

const root = document.getElementById('root');
if (!root) throw new Error('Root element "#root" not found');

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
