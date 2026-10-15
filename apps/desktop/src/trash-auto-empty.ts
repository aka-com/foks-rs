/**
 * This device's choice for emptying vault Trash automatically. It is off
 * unless chosen: entries deleted longer ago than the chosen number of days
 * are then removed permanently, which no other device or client is told of.
 */

import {
  readStorage,
  STORAGE_KEYS,
  writeStorage,
} from './platform/device-storage';

/** Days an entry stays in Trash before automatic emptying; `null` is off. */
export type TrashAutoEmpty = 30 | 90 | null;

export const TRASH_AUTO_EMPTY_CHOICES: readonly TrashAutoEmpty[] = [
  null,
  30,
  90,
];

export function storedTrashAutoEmpty(): TrashAutoEmpty {
  const stored = readStorage(STORAGE_KEYS.trashAutoEmptyDays);
  return stored === '30' ? 30 : stored === '90' ? 90 : null;
}

export function rememberTrashAutoEmpty(days: TrashAutoEmpty): void {
  // Restricted storage leaves automatic emptying off.
  writeStorage(
    STORAGE_KEYS.trashAutoEmptyDays,
    days === null ? null : String(days),
  );
}
