/**
 * The items the ⌘K palette lists before a query is typed: the ten most
 * recently opened or copied on this device, and the ones marked as favorites.
 *
 * Both lists hold opaque identifiers only, an item's `StoreRef` and its dirent
 * id, under the `foks.recentFavorites` `localStorage` key. Names and paths are
 * decrypted vault metadata and are not written to storage, which persists
 * while the app is locked. The palette resolves each identifier against the
 * live snapshot when it draws, so an item that was removed, moved to Trash or
 * is not in the snapshot yet is omitted rather than shown by an old name. A
 * move or rename keeps the dirent id, so the entry follows the item. The
 * records are per device and are not shared with other devices.
 */

import { useSyncExternalStore } from 'react';

import type { Item, StoreRef } from './model';

const STORAGE_KEY = 'foks.recentFavorites';

/** How many recently used items are kept. */
export const RECENT_LIMIT = 10;

/** Favorites beyond this many are dropped, oldest first. */
const FAVORITE_LIMIT = 100;

/** One item, by the identifiers that survive a move or a rename. */
export interface ItemRef {
  store: StoreRef;
  dirent: string;
}

export interface RecentFavorites {
  /** Most recent first. */
  recent: readonly ItemRef[];
  /** Most recently added first. */
  favorites: readonly ItemRef[];
}

const EMPTY: RecentFavorites = { recent: [], favorites: [] };

/** The identifiers of a listed item, or `null` when it has no dirent id. */
export function itemRefOf(
  item: Pick<Item, 'store' | 'direntId'>,
): ItemRef | null {
  return item.direntId ? { store: item.store, dirent: item.direntId } : null;
}

export function sameItemRef(left: ItemRef, right: ItemRef): boolean {
  return left.store === right.store && left.dirent === right.dirent;
}

function refs(value: unknown, limit: number): ItemRef[] {
  if (!Array.isArray(value)) return [];
  const result: ItemRef[] = [];
  for (const entry of value) {
    if (!entry || typeof entry !== 'object') continue;
    const { store, dirent } = entry as Record<string, unknown>;
    if (typeof store !== 'string' || typeof dirent !== 'string') continue;
    if (!store || !dirent) continue;
    const ref = { store, dirent };
    if (result.some((seen) => sameItemRef(seen, ref))) continue;
    result.push(ref);
    if (result.length >= limit) break;
  }
  return result;
}

function parse(raw: string | null): RecentFavorites {
  if (!raw) return EMPTY;
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed))
      return EMPTY;
    const record = parsed as Record<string, unknown>;
    return {
      recent: refs(record.recent, RECENT_LIMIT),
      favorites: refs(record.favorites, FAVORITE_LIMIT),
    };
  } catch {
    return EMPTY;
  }
}

function rawRecords(): string | null {
  try {
    if (typeof window === 'undefined') return null;
    return window.localStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
}

function writeAll(value: RecentFavorites): void {
  try {
    if (typeof window === 'undefined') return;
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(value));
  } catch {
    return;
  }
}

let cached: { raw: string | null; value: RecentFavorites } | undefined;
const listeners = new Set<() => void>();

/**
 * The current lists. The value is reused while the stored text is unchanged,
 * so React can compare snapshots by identity.
 */
export function recentFavorites(): RecentFavorites {
  const raw = rawRecords();
  if (!cached || cached.raw !== raw) cached = { raw, value: parse(raw) };
  return cached.value;
}

export function subscribeRecentFavorites(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** The current lists, re-read whenever one of them changes. */
export function useRecentFavorites(): RecentFavorites {
  return useSyncExternalStore(
    subscribeRecentFavorites,
    recentFavorites,
    () => EMPTY,
  );
}

function update(change: (current: RecentFavorites) => RecentFavorites): void {
  writeAll(change(recentFavorites()));
  for (const listener of listeners) listener();
}

/** Moves the item to the head of Recent, keeping {@link RECENT_LIMIT}. */
export function rememberRecent(ref: ItemRef | null): void {
  if (!ref) return;
  update((current) => ({
    ...current,
    recent: [
      ref,
      ...current.recent.filter((seen) => !sameItemRef(seen, ref)),
    ].slice(0, RECENT_LIMIT),
  }));
}

export function isFavorite(lists: RecentFavorites, ref: ItemRef | null) {
  return Boolean(ref && lists.favorites.some((seen) => sameItemRef(seen, ref)));
}

/** Adds the item to Favorites or removes it; returns whether it is now one. */
export function toggleFavorite(ref: ItemRef | null): boolean {
  if (!ref) return false;
  const added = !isFavorite(recentFavorites(), ref);
  update((current) => ({
    ...current,
    favorites: added
      ? [ref, ...current.favorites].slice(0, FAVORITE_LIMIT)
      : current.favorites.filter((seen) => !sameItemRef(seen, ref)),
  }));
  return added;
}
