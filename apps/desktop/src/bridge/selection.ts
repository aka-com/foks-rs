import type { Bridge } from './contract';
import { tauriBridge } from './tauri';

declare global {
  interface Window {
    /** Injected by the Tauri runtime even with `withGlobalTauri: false`. */
    __TAURI_INTERNALS__?: unknown;
  }
}

export function isNativeHost(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

export function mockRequested(): boolean {
  return import.meta.env?.VITE_FOKS_MOCK === '1';
}

export async function selectBridge(): Promise<Bridge> {
  // A development Tauri window still exercises the real command boundary.
  if (isNativeHost()) return tauriBridge;
  // Both conditions are compile-time constants in Vite. Ordinary production
  // builds eliminate this branch and its fixture chunk completely.
  if (
    import.meta.env?.VITE_FOKS_MOCK === '1' ||
    import.meta.env?.DEV === true
  ) {
    const { mockBridge } = await import('../mock-bridge');
    return mockBridge();
  }
  throw new Error('FOKS desktop host environment is required.');
}
