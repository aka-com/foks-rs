import { useSyncExternalStore } from 'react';

export type Appearance = 'light' | 'dark' | 'system';
const KEY = 'appearance';
const CHANGE = 'foks-appearance-change';
const valid = (value: unknown): Appearance =>
  value === 'dark' || value === 'system' ? value : 'light';
let session: Appearance | undefined;
export function storedAppearance(): Appearance {
  if (session) return session;
  try {
    return valid(window.localStorage.getItem(KEY));
  } catch {
    return 'light';
  }
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
  try {
    window.localStorage.setItem(KEY, session);
  } catch {
    /* Session only. */
  }
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
    if (event.key !== KEY && event.key !== null) return;
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
