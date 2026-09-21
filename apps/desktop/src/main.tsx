/**
 * The FOKS desktop frontend's entry point.
 *
 * Mounts the main application shell with state and commands provided
 * through `src/bridge.ts`.
 */

import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from '/src/app-root';
import { mountAppearance } from './appearance';
import './styles/index.css';

const stopAppearance = mountAppearance();
if (import.meta.hot) import.meta.hot.dispose(stopAppearance);

const root = document.getElementById('root');
if (!root) throw new Error('Root element "#root" not found');

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
