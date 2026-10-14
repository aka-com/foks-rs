/**
 * Exposure reviews: the items a removed member, a lowered member or a
 * removed device could read, kept so each can be checked off after the
 * change.
 *
 * Removing a member rekeys the team, and removing a device ends its access to
 * the account; both stop later reads only. A credential the party already
 * read stays valid wherever it is used until it is changed there. Before the
 * write, the sheet lists what the party could read; once the write succeeds,
 * {@link recordExposure} keeps that list as a review, and a band on the team
 * page or on Devices leads to it until every entry is marked.
 *
 * The list is a lower bound. It is computed from this account's catalog, so
 * an item this account cannot list is not in it; Trash entries and folders
 * are not in it either. A member's list is the items whose read role the
 * member's role admits; a device's list is every item in every store of its
 * account, since the device held the account's keys.
 *
 * A review is stored under the `foks.exposureReviews` `localStorage` key,
 * which persists while the app is locked. Each entry holds an item's
 * `StoreRef`, its dirent id and its version when the access ended, and the
 * mark the user gave it. Item names and paths are decrypted vault metadata
 * and are not written to storage; the review sheet resolves them from the
 * live snapshot. The review also holds the party's id (a party id or a device
 * key id) and its display name (a username or a device name), which the
 * server holds in its own records; the removed party is no longer in the
 * roster or the device list to resolve it from.
 *
 * An entry is "Changed since removal" once the item at its dirent id has a
 * higher version. That is not proof of rotation: any write raises the
 * version, so the user confirms each entry as rotated or marks it not
 * needed. A move or rename writes the item under a new dirent id, so an
 * entry whose item was moved reads as no longer listed. The records are per
 * device and are not shared with other devices.
 */

import { useSyncExternalStore } from 'react';

import {
  admits,
  catalog,
  combineReadiness,
  groupDetailReadiness,
  inventoryReadiness,
  isTrashPath,
  itemsReadiness,
  kindOf,
  readersOf,
} from './model';
import type {
  AgentSnapshot,
  CollectionReadiness,
  Item,
  Party,
  RoleWire,
  StoreRef,
} from './model';

const STORAGE_KEY = 'foks.exposureReviews';

/** Reviews beyond this many are dropped, oldest first. */
export const REVIEW_LIMIT = 50;

/** What ended the party's access. */
export type ExposureCause = 'removed' | 'lowered' | 'device-removed';

/** What the user said about one entry. */
export type ExposureMark = 'rotated' | 'not-needed';

export interface ExposureEntry {
  store: StoreRef;
  dirent: string;
  /** The item's version when the access ended. */
  version: number;
  mark?: ExposureMark;
}

export interface ExposureReview {
  /** {@link reviewId} of `scope` and `party`. */
  id: string;
  /** The team a member left, or the account store a device left. */
  scope: StoreRef;
  /** The member's party id, or the device's key id. */
  party: string;
  /** The username or device name, for the band and the sheet title. */
  name: string;
  cause: ExposureCause;
  /** When the review was recorded, in milliseconds since the epoch. */
  at: number;
  entries: readonly ExposureEntry[];
}

/** One review per party and scope: a later change to it is merged in. */
export function reviewId(scope: StoreRef, party: string): string {
  return `${scope}|${party}`;
}

/* ------------------------------------------------------------ exposure -- */

/** The catalog items of `party`'s team that its role admits. */
export function memberReads(snapshot: AgentSnapshot, party: Party): Item[] {
  return catalog(snapshot).filter(
    (item) =>
      item.store === party.store &&
      Boolean(
        readersOf(snapshot, item)?.some(
          (candidate) => candidate.party_id_hex === party.party_id_hex,
        ),
      ),
  );
}

/**
 * The items `party` reads now and would not read at `destination`: what a
 * lower role takes away from later reads but not from what was already read.
 */
export function loweredReads(
  snapshot: AgentSnapshot,
  party: Party,
  destination: RoleWire,
): Item[] {
  return memberReads(snapshot, party).filter(
    (item) => !admits(destination, item.read),
  );
}

/** The stores an account holds: its own and its teams on the same server. */
function accountStoreRefs(
  snapshot: AgentSnapshot,
  account: { server: string; account: string },
): StoreRef[] {
  return snapshot.stores
    .filter(
      (store) =>
        store.server === account.server && store.account === account.account,
    )
    .map((store) => store.id);
}

/** Every catalog item in every store of the account a device belonged to. */
export function accountReads(
  snapshot: AgentSnapshot,
  account: { server: string; account: string },
): Item[] {
  const stores = new Set(accountStoreRefs(snapshot, account));
  return catalog(snapshot).filter((item) => stores.has(item.store));
}

/** Whether the team's items and roster are read, so a member's list is whole. */
export function memberExposureReadiness(
  snapshot: AgentSnapshot,
  store: StoreRef,
): CollectionReadiness {
  return combineReadiness([
    itemsReadiness(snapshot, store),
    groupDetailReadiness(snapshot, store, 'roster'),
  ]);
}

/**
 * Whether the account's teams and every one of its stores' items are read,
 * so a device's list is whole.
 */
export function accountExposureReadiness(
  snapshot: AgentSnapshot,
  account: { server: string; account: string },
): CollectionReadiness {
  return combineReadiness([
    inventoryReadiness(snapshot, 'teams', account.server),
    ...accountStoreRefs(snapshot, account).map((store) =>
      itemsReadiness(snapshot, store),
    ),
  ]);
}

/** Passwords first, then Documents, each by store and path. */
export function exposureOrder(items: readonly Item[]): Item[] {
  const rank = (item: Item) => (kindOf(item) === 'Password' ? 0 : 1);
  return [...items].sort(
    (left, right) =>
      rank(left) - rank(right) ||
      left.store.localeCompare(right.store) ||
      left.path.localeCompare(right.path),
  );
}

/** The entries a review keeps for `items`: ids and versions only. */
export function entriesOf(items: readonly Item[]): ExposureEntry[] {
  return items
    .filter((item) => item.direntId)
    .map((item) => ({
      store: item.store,
      dirent: item.direntId,
      version: item.version,
    }));
}

/**
 * - `rotated` / `not-needed`: the user's mark.
 * - `missing`: no item in the snapshot has this dirent id; it was moved,
 *   renamed or removed, or its store is not listed now.
 * - `changed`: the item's version is above the one at removal.
 * - `unchanged`: the item is at the version it had at removal.
 */
export type ExposureState = ExposureMark | 'missing' | 'changed' | 'unchanged';

/** The live item an entry names, if the snapshot lists it outside Trash. */
export function entryItem(
  snapshot: AgentSnapshot,
  entry: ExposureEntry,
): Item | undefined {
  return snapshot.items.find(
    (item) =>
      item.store === entry.store &&
      item.direntId === entry.dirent &&
      !isTrashPath(item.path),
  );
}

export function entryState(
  snapshot: AgentSnapshot,
  entry: ExposureEntry,
): ExposureState {
  if (entry.mark) return entry.mark;
  const item = entryItem(snapshot, entry);
  if (!item) return 'missing';
  return item.version > entry.version ? 'changed' : 'unchanged';
}

/** How many of a review's entries the user has not marked. */
export function openEntries(review: ExposureReview): number {
  return review.entries.filter((entry) => !entry.mark).length;
}

/* ------------------------------------------------------------- storage -- */

const CAUSES: readonly ExposureCause[] = [
  'removed',
  'lowered',
  'device-removed',
];
const MARKS: readonly ExposureMark[] = ['rotated', 'not-needed'];

function entries(value: unknown): ExposureEntry[] {
  if (!Array.isArray(value)) return [];
  const result: ExposureEntry[] = [];
  for (const entry of value) {
    if (!entry || typeof entry !== 'object') continue;
    const { store, dirent, version, mark } = entry as Record<string, unknown>;
    if (typeof store !== 'string' || !store) continue;
    if (typeof dirent !== 'string' || !dirent) continue;
    if (typeof version !== 'number' || !Number.isSafeInteger(version)) continue;
    if (result.some((seen) => sameEntry(seen, { store, dirent }))) continue;
    result.push({
      store,
      dirent,
      version,
      ...(MARKS.includes(mark as ExposureMark)
        ? { mark: mark as ExposureMark }
        : {}),
    });
  }
  return result;
}

function parse(raw: string | null): readonly ExposureReview[] {
  if (!raw) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    const result: ExposureReview[] = [];
    for (const value of parsed) {
      if (!value || typeof value !== 'object') continue;
      const record = value as Record<string, unknown>;
      const { scope, party, name, cause, at } = record;
      if (typeof scope !== 'string' || !scope) continue;
      if (typeof party !== 'string' || !party) continue;
      if (typeof name !== 'string') continue;
      if (!CAUSES.includes(cause as ExposureCause)) continue;
      if (typeof at !== 'number' || !Number.isFinite(at)) continue;
      const id = reviewId(scope, party);
      if (result.some((seen) => seen.id === id)) continue;
      result.push({
        id,
        scope,
        party,
        name,
        cause: cause as ExposureCause,
        at,
        entries: entries(record.entries),
      });
      if (result.length >= REVIEW_LIMIT) break;
    }
    return result;
  } catch {
    return [];
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

/** Whether the reviews were stored; blocked or full storage refuses them. */
function writeAll(value: readonly ExposureReview[]): boolean {
  try {
    if (typeof window === 'undefined') return false;
    window.localStorage.setItem(
      STORAGE_KEY,
      // The id is derived from scope and party, so it is not stored.
      JSON.stringify(
        value.map((review) => ({
          scope: review.scope,
          party: review.party,
          name: review.name,
          cause: review.cause,
          at: review.at,
          entries: review.entries,
        })),
      ),
    );
    return true;
  } catch {
    return false;
  }
}

let cached:
  { raw: string | null; value: readonly ExposureReview[] } | undefined;
const listeners = new Set<() => void>();

/**
 * The stored reviews, most recent first. The value is reused while the
 * stored text is unchanged, so React can compare snapshots by identity.
 */
export function exposureReviews(): readonly ExposureReview[] {
  const raw = rawRecords();
  if (!cached || cached.raw !== raw) cached = { raw, value: parse(raw) };
  return cached.value;
}

export function subscribeExposureReviews(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

const NONE: readonly ExposureReview[] = [];

/** The stored reviews, re-read whenever one of them changes. */
export function useExposureReviews(): readonly ExposureReview[] {
  return useSyncExternalStore(
    subscribeExposureReviews,
    exposureReviews,
    () => NONE,
  );
}

function update(
  change: (current: readonly ExposureReview[]) => readonly ExposureReview[],
): boolean {
  const stored = writeAll(change(exposureReviews()));
  for (const listener of listeners) listener();
  return stored;
}

function sameEntry(
  left: Pick<ExposureEntry, 'store' | 'dirent'>,
  right: Pick<ExposureEntry, 'store' | 'dirent'>,
): boolean {
  return left.store === right.store && left.dirent === right.dirent;
}

/**
 * Keeps a review of `entries` for the party, after its access ended. A
 * review already kept for the same party and scope keeps its entries, with
 * their versions and marks, and gains the new ones; the review moves to the
 * front. Nothing is stored for an empty list. Returns whether the review was
 * stored.
 */
export function recordExposure(
  review: Omit<ExposureReview, 'id' | 'at'>,
  at = Date.now(),
): boolean {
  if (!review.entries.length) return true;
  const id = reviewId(review.scope, review.party);
  return update((current) => {
    const previous = current.find((seen) => seen.id === id);
    const kept = previous?.entries ?? [];
    const merged = [
      ...kept,
      ...review.entries.filter(
        (entry) => !kept.some((seen) => sameEntry(seen, entry)),
      ),
    ];
    return [
      { ...review, id, at, entries: merged },
      ...current.filter((seen) => seen.id !== id),
    ].slice(0, REVIEW_LIMIT);
  });
}

/** Sets or, with `null`, clears the mark on one entry of a review. */
export function markExposure(
  id: string,
  entry: Pick<ExposureEntry, 'store' | 'dirent'>,
  mark: ExposureMark | null,
): void {
  update((current) =>
    current.map((review) =>
      review.id !== id
        ? review
        : {
            ...review,
            entries: review.entries.map((seen) =>
              sameEntry(seen, entry)
                ? {
                    store: seen.store,
                    dirent: seen.dirent,
                    version: seen.version,
                    ...(mark ? { mark } : {}),
                  }
                : seen,
            ),
          },
    ),
  );
}

/** Removes a review. */
export function dismissExposure(id: string): void {
  update((current) => current.filter((review) => review.id !== id));
}
