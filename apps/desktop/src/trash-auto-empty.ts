/**
 * This device's choice for emptying vault Trash automatically. It is off
 * unless chosen: entries deleted longer ago than the chosen number of days
 * are then removed permanently, which no other device or client is told of.
 */

/** Days an entry stays in Trash before automatic emptying; `null` is off. */
export type TrashAutoEmpty = 30 | 90 | null;

export const TRASH_AUTO_EMPTY_CHOICES: readonly TrashAutoEmpty[] = [
  null,
  30,
  90,
];

const STORAGE_KEY = 'trashAutoEmptyDays';

export function storedTrashAutoEmpty(): TrashAutoEmpty {
  try {
    if (typeof window === 'undefined') return null;
    const stored = window.localStorage.getItem(STORAGE_KEY);
    return stored === '30' ? 30 : stored === '90' ? 90 : null;
  } catch {
    return null;
  }
}

export function rememberTrashAutoEmpty(days: TrashAutoEmpty): void {
  try {
    if (typeof window === 'undefined') return;
    if (days === null) window.localStorage.removeItem(STORAGE_KEY);
    else window.localStorage.setItem(STORAGE_KEY, String(days));
  } catch {
    // Restricted storage leaves automatic emptying off.
  }
}
