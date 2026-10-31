/**
 * This device's choice for emptying vault Trash automatically. It is off
 * unless chosen: entries deleted longer ago than the chosen number of days
 * are then removed permanently, which no other device or client is told of.
 *
 * One `localStorage` value, `trashAutoEmptyDays`, `'30'` or `'90'`, and beside
 * it, while the choice is on, `trashAutoEmptySeen`: when this device first
 * listed each entry in Trash. Reads and writes are guarded because storage is
 * unavailable in restricted contexts; an unreadable value leaves automatic
 * emptying off.
 */

const KEY = 'trashAutoEmptyDays';
const SEEN_KEY = 'trashAutoEmptySeen';

/** Days an entry stays in Trash before automatic emptying; `null` is off. */
export type TrashAutoEmpty = 30 | 90 | null;

export const TRASH_AUTO_EMPTY_CHOICES: readonly TrashAutoEmpty[] = [
  null,
  30,
  90,
];

export function storedTrashAutoEmpty(): TrashAutoEmpty {
  try {
    const stored =
      typeof window === 'undefined' ? null : window.localStorage.getItem(KEY);
    return stored === '30' ? 30 : stored === '90' ? 90 : null;
  } catch {
    return null;
  }
}

export function rememberTrashAutoEmpty(days: TrashAutoEmpty): void {
  try {
    if (typeof window === 'undefined') return;
    if (days === null) {
      window.localStorage.removeItem(KEY);
      window.localStorage.removeItem(SEEN_KEY);
    } else window.localStorage.setItem(KEY, String(days));
  } catch {
    // Restricted storage leaves automatic emptying off.
  }
}

/** The record key of a Trash entry: its store, path and dirent id. */
export function trashSeenKey(item: {
  store: string;
  path: string;
  direntId: string;
}): string {
  return JSON.stringify([item.store, item.path, item.direntId]);
}

function readSeen(): Record<string, number> {
  try {
    const stored =
      typeof window === 'undefined'
        ? null
        : window.localStorage.getItem(SEEN_KEY);
    const parsed: unknown = stored ? JSON.parse(stored) : {};
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed))
      return {};
    return Object.fromEntries(
      Object.entries(parsed).filter(
        (entry): entry is [string, number] =>
          Number.isSafeInteger(entry[1]) && entry[1] >= 0,
      ),
    );
  } catch {
    return {};
  }
}

/**
 * When this device first listed each Trash entry, which automatic emptying
 * counts an entry's age from when its stamp is earlier. `keys` are the
 * entries listed now (see {@link trashSeenKey}): one not recorded yet is
 * recorded at `now`, and a recorded one no longer listed is dropped, so the
 * record holds only what Trash holds. Without storage every entry counts as
 * first seen now, which keeps it in Trash.
 */
export function rememberTrashEntriesSeen(
  keys: Iterable<string>,
  now: number,
): ReadonlyMap<string, number> {
  const stored = readSeen();
  const seen = new Map<string, number>();
  for (const key of keys) seen.set(key, Math.min(stored[key] ?? now, now));
  try {
    if (typeof window !== 'undefined')
      window.localStorage.setItem(
        SEEN_KEY,
        JSON.stringify(Object.fromEntries(seen)),
      );
  } catch {
    // Restricted storage: the record lasts for this call only.
  }
  return seen;
}
