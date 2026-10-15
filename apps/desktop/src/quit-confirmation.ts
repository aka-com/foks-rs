/**
 * This device's Ask before quitting preference. The native close guard makes
 * the decision, so the exit guard reports the stored value to it whenever it
 * mounts against a native bridge and whenever the value changes. Until then
 * the guard asks on every close and quit, which is also the default here.
 */

import { useSyncExternalStore } from 'react';

import type { QuitConfirmation } from './bridge';
import {
  readStorage,
  STORAGE_KEYS,
  writeStorage,
} from './platform/device-storage';

const CHANGE = 'foks-quit-confirmation-change';

let session: QuitConfirmation | undefined;

export function storedQuitConfirmation(): QuitConfirmation {
  if (session) return session;
  return readStorage(STORAGE_KEYS.quitConfirmation) === 'when-pending'
    ? 'when-pending'
    : 'always';
}

export function rememberQuitConfirmation(value: QuitConfirmation): void {
  session = value;
  // A refused write keeps the choice for this session only.
  writeStorage(
    STORAGE_KEYS.quitConfirmation,
    value === 'when-pending' ? value : null,
  );
  window.dispatchEvent(new Event(CHANGE));
}

const subscribe = (notify: () => void): (() => void) => {
  window.addEventListener(CHANGE, notify);
  return () => window.removeEventListener(CHANGE, notify);
};

export function useQuitConfirmation(): QuitConfirmation {
  return useSyncExternalStore(
    subscribe,
    storedQuitConfirmation,
    () => 'always',
  );
}
