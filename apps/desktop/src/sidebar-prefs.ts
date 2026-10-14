/**
 * The sidebar's persisted width preference.
 *
 * One boolean in `localStorage`, `sideCollapsed`, encoded as `'1'` / `'0'` and
 * defaulting to false. Storage is unavailable in restricted contexts, which
 * reads as the default, and a display preference is never worth failing the
 * shell for.
 */

import {
  readStorage,
  STORAGE_KEYS,
  writeStorage,
} from './platform/device-storage';

/** The stored collapse preference. False (expanded) when unset or unreadable. */
export function storedSideCollapsedPref(): boolean {
  return readStorage(STORAGE_KEYS.sideCollapsed) === '1';
}

/** Persists the collapse preference. */
export function rememberSideCollapsed(collapsed: boolean): void {
  // Storage may be unavailable; the preference then lasts for this session only.
  writeStorage(STORAGE_KEYS.sideCollapsed, collapsed ? '1' : '0');
}
