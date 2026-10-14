/**
 * The items the ⌘K palette lists before a query is typed: the ten most
 * recently opened or copied on this device, and up to 100 marked as
 * favorites, all of which the palette lists.
 *
 * Both lists hold an item's `StoreRef` and its dirent id, under the
 * `foks.recentFavorites` `localStorage` key, which persists while the app is
 * locked. A `StoreRef` names the server profile and the account and, for a
 * team, the team's alias and id. Item names and paths are decrypted vault
 * metadata and are not written to storage. The palette resolves each entry
 * against the live snapshot when it draws, so an item that was removed,
 * moved to Trash or is not in the snapshot yet is omitted rather than shown
 * by an old name.
 *
 * A dirent id names the entry at one path, not the item. A move or a rename
 * writes the item under a new dirent id at its destination and leaves a
 * tombstone with the old id at the source; an item later created at the
 * source path takes over that id. A move made in this app is followed: the
 * move sheets call {@link noteMoved}, and once a snapshot holds the item at
 * its destination, {@link followMoves} gives its entries the new id. A move
 * made on another device is not followed, so the entry is omitted, and an
 * item later created at the old path is listed in its place, as is an item
 * created at the path of one that was removed. The records are per device
 * and are not shared with other devices.
 */

import { useSyncExternalStore } from 'react';

import type { Item, StoreRef } from './model';

const STORAGE_KEY = 'foks.recentFavorites';

/** How many recently used items are kept. */
export const RECENT_LIMIT = 10;

/** Favorites beyond this many are dropped, oldest first. */
export const FAVORITE_LIMIT = 100;

/** One item, by its store and the dirent id of its current path. */
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

/**
 * Adds the item to Favorites, dropping the oldest past
 * {@link FAVORITE_LIMIT}, or removes it.
 */
export function toggleFavorite(ref: ItemRef | null): void {
  if (!ref) return;
  const added = !isFavorite(recentFavorites(), ref);
  update((current) => ({
    ...current,
    favorites: added
      ? [ref, ...current.favorites].slice(0, FAVORITE_LIMIT)
      : current.favorites.filter((seen) => !sameItemRef(seen, ref)),
  }));
}

/**
 * Moves made in this app whose destination id no snapshot has shown yet. The
 * move response does not carry the new dirent id, so it is read from the
 * refreshed snapshot. Kept in memory only: after a move the app quits before
 * seeing, the entries keep the source's id, as after a move made elsewhere.
 */
const pendingMoves: { from: ItemRef; path: string }[] = [];

/** How many unsettled moves are kept; the oldest is dropped past this. */
const PENDING_MOVE_LIMIT = 100;

/**
 * Records that `item` was moved or renamed to `path` in its store, when
 * Recent or Favorites lists it, for {@link followMoves} to settle.
 */
export function noteMoved(
  item: Pick<Item, 'store' | 'direntId'>,
  path: string,
): void {
  const from = itemRefOf(item);
  if (!from) return;
  const { recent, favorites } = recentFavorites();
  if (![...recent, ...favorites].some((seen) => sameItemRef(seen, from)))
    return;
  pendingMoves.push({ from, path });
  if (pendingMoves.length > PENDING_MOVE_LIMIT) pendingMoves.shift();
}

/**
 * Gives each entry recorded by {@link noteMoved} the dirent id of the item
 * that `items` holds at the move's destination, keeping its place in the
 * list. A move whose destination is not in `items` yet stays recorded.
 */
export function followMoves(
  items: readonly Pick<Item, 'store' | 'path' | 'direntId'>[],
): void {
  if (!pendingMoves.length) return;
  const settled: { from: ItemRef; to: ItemRef }[] = [];
  for (let index = 0; index < pendingMoves.length;) {
    const { from, path } = pendingMoves[index];
    const moved = items.find(
      (item) => item.store === from.store && item.path === path,
    );
    const to = moved ? itemRefOf(moved) : null;
    if (!to) {
      index += 1;
      continue;
    }
    pendingMoves.splice(index, 1);
    settled.push({ from, to });
  }
  if (!settled.length) return;
  const follow = (list: readonly ItemRef[]): ItemRef[] => {
    const result: ItemRef[] = [];
    for (const ref of list) {
      const next =
        settled.find(({ from }) => sameItemRef(from, ref))?.to ?? ref;
      if (!result.some((seen) => sameItemRef(seen, next))) result.push(next);
    }
    return result;
  };
  update((current) => ({
    recent: follow(current.recent),
    favorites: follow(current.favorites),
  }));
}
