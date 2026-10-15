/**
 * The FOKS desktop frontend's entry point.
 *
 * Mounts the main application shell with state and commands provided
 * through `src/bridge.ts`, and records renderer faults for Copy diagnostics
 * (`src/diagnostics/renderer-fault.ts`).
 */

import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './app-root';
import { WindowFaultBoundary } from './app/fault-boundaries';
import { mountAppearance } from './platform/appearance';
import {
  listenForWindowFaults,
  rendererRootOptions,
} from './diagnostics/renderer-fault';
import './styles/index.css';

const stopAppearance = mountAppearance();
if (import.meta.hot) import.meta.hot.dispose(stopAppearance);
const stopFaults = listenForWindowFaults(window);
if (import.meta.hot) import.meta.hot.dispose(stopFaults);

const root = document.getElementById('root');
if (!root) throw new Error('Root element "#root" not found');

// The app's own boundary sits inside its exit guard; this one catches a
// failure of the guard or of the boot hooks above it.
createRoot(root, rendererRootOptions()).render(
  <StrictMode>
    <WindowFaultBoundary label="app">
      <App />
    </WindowFaultBoundary>
  </StrictMode>,
);
