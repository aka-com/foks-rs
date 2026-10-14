import { useSyncExternalStore } from 'react';

import {
  readStorage,
  STORAGE_KEYS,
  writeStorage,
} from './platform/device-storage';

export type Appearance = 'light' | 'dark' | 'system';
const KEY = STORAGE_KEYS.appearance;
const CHANGE = 'foks-appearance-change';
const valid = (value: unknown): Appearance =>
  value === 'dark' || value === 'system' ? value : 'light';
let session: Appearance | undefined;
export function storedAppearance(): Appearance {
  if (session) return session;
  return valid(readStorage(KEY));
}
export function applyAppearance(value: Appearance): void {
  document.documentElement.dataset.theme =
    value === 'system'
      ? window.matchMedia?.('(prefers-color-scheme: dark)').matches
        ? 'dark'
        : 'light'
      : value;
}
export function setAppearance(value: Appearance): void {
  session = valid(value);
  // A refused write keeps the choice for this session only.
  writeStorage(KEY, session);
  applyAppearance(session);
  window.dispatchEvent(new Event(CHANGE));
}
const subscribe = (notify: () => void): (() => void) => {
  window.addEventListener(CHANGE, notify);
  return () => window.removeEventListener(CHANGE, notify);
};
export function useAppearance() {
  return useSyncExternalStore(subscribe, storedAppearance, () => 'light');
}
/** Keep System appearance current, and synchronize preference changes across windows. */
export function mountAppearance(): () => void {
  applyAppearance(storedAppearance());
  const media = window.matchMedia?.('(prefers-color-scheme: dark)');
  const changed = () => applyAppearance(storedAppearance());
  const storage = (event: StorageEvent) => {
    if (event.key !== KEY.name && event.key !== null) return;
    session = undefined;
    changed();
    window.dispatchEvent(new Event(CHANGE));
  };
  media?.addEventListener('change', changed);
  window.addEventListener('storage', storage);
  return () => {
    media?.removeEventListener('change', changed);
    window.removeEventListener('storage', storage);
  };
}
