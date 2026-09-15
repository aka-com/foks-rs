/**
 * The sidebar's persisted width preference.
 *
 * Two booleans in `localStorage`, encoded as `'1'` / `'0'` and defaulting to
 * false: `sideCollapsed` is the rail's width, `sidePinned` records that the
 * reader chose it, which suppresses hover expansion. Reads and writes are
 * guarded because storage is unavailable in restricted contexts, and a display
 * preference is never worth failing the shell for.
 */

function read(key: string): boolean {
  try {
    return (
      typeof window !== 'undefined' && window.localStorage.getItem(key) === '1'
    );
  } catch {
    return false;
  }
}

function write(key: string, value: boolean): void {
  try {
    if (typeof window !== 'undefined')
      window.localStorage.setItem(key, value ? '1' : '0');
  } catch {
    // Storage is unavailable; the preference lasts for this session only.
  }
}

/** The stored collapse preference. False (expanded) when unset or unreadable. */
export function storedSideCollapsedPref(): boolean {
  return read('sideCollapsed');
}

/** Persists the collapse preference. */
export function rememberSideCollapsed(collapsed: boolean): void {
  write('sideCollapsed', collapsed);
}

/** The stored pin preference. A pinned rail expands on focus but not hover. */
export function storedSidePinnedPref(): boolean {
  return read('sidePinned');
}

/** Persists the pin preference. */
export function rememberSidePinned(pinned: boolean): void {
  write('sidePinned', pinned);
}
