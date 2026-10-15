/**
 * Exposure reviews: the items a removed member, a lowered member or a
 * removed device could read, kept so each can be checked off after the
 * change.
 *
 * Removing a member rekeys the team, and removing a device ends its access to
 * the account; both stop later reads only. A credential the party already
 * read stays valid wherever it is used until it is changed there. Before the
 * write, the sheet lists what the party could read; once the write succeeds,
 * or fails without showing whether it was applied, {@link withExposureReview}
 * keeps that list as a review, and a band on the team page or on Devices
 * leads to it until every entry is marked.
 *
 * The list is a lower bound. It is computed from this account's catalog, so
 * an item this account cannot list is not in it; Trash entries and folders
 * are not in it either. A member's list is the items whose read role the
 * member's role admits; a device's list is every item in every store of its
 * account, since the device held the account's keys. A store whose items
 * cannot be listed is left out of a device's list and named in the sheet.
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
  storeAvailability,
} from './model';
import type {
  AgentSnapshot,
  CollectionReadiness,
  Item,
  Party,
  RoleWire,
  Store,
  StoreRef,
} from './model';
import { failureOutcome } from './operation-outcome';
import { STORAGE_KEYS, storedRecord } from './platform/device-storage';

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

/**
 * The stores an account holds: its own and its teams on the same server. A
 * team whose setup has not completed (`active: false`) is left out: the
 * catalog does not read its items, and no device of the account read them.
 */
function accountStores(
  snapshot: AgentSnapshot,
  account: { server: string; account: string },
): Store[] {
  return snapshot.stores.filter(
    (store) =>
      store.server === account.server &&
      store.account === account.account &&
      !(store.kind === 'team' && !store.active),
  );
}

/** What a removed device could read, as far as this account can list it. */
export interface AccountExposure {
  /**
   * `loading` while the account's teams or any of its stores' items are
   * still being read; `unavailable` when no store's items could be listed;
   * otherwise `ready`, with the items of the stores that were listed.
   */
  readiness: CollectionReadiness;
  /** Every catalog item in the listed stores; empty unless `ready`. */
  items: Item[];
  /** The stores whose items could not be listed, so are not in `items`. */
  unlisted: StoreRef[];
  /** The account's team list could not be read, so a team may be missing. */
  teamsUnlisted: boolean;
}

/**
 * Every catalog item in every store of the account a device belonged to,
 * since the device held the account's keys. A store whose items cannot be
 * listed is named in `unlisted` rather than withholding the other stores'
 * items; the list is a lower bound either way.
 */
export function accountExposure(
  snapshot: AgentSnapshot,
  account: { server: string; account: string },
): AccountExposure {
  const teams = inventoryReadiness(snapshot, 'teams', account.server);
  let loading = teams === 'loading';
  const listed = new Set<StoreRef>();
  const unlisted: StoreRef[] = [];
  for (const store of accountStores(snapshot, account)) {
    if (listed.has(store.id) || unlisted.includes(store.id)) continue;
    const readiness = itemsReadiness(snapshot, store.id);
    if (readiness === 'loading') loading = true;
    // `catalog` leaves out the items of a store it cannot read, so such a
    // store is unlisted even when its items were read.
    else if (
      readiness === 'ready' &&
      storeAvailability(snapshot, store).available
    )
      listed.add(store.id);
    else unlisted.push(store.id);
  }
  const teamsUnlisted = teams === 'unavailable';
  if (loading)
    return { readiness: 'loading', items: [], unlisted: [], teamsUnlisted };
  if (!listed.size)
    return { readiness: 'unavailable', items: [], unlisted, teamsUnlisted };
  return {
    readiness: 'ready',
    items: catalog(snapshot).filter((item) => listed.has(item.store)),
    unlisted,
    teamsUnlisted,
  };
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

function encode(value: readonly ExposureReview[]): string {
  // The id is derived from scope and party, so it is not stored.
  return JSON.stringify(
    value.map((review) => ({
      scope: review.scope,
      party: review.party,
      name: review.name,
      cause: review.cause,
      at: review.at,
      entries: review.entries,
    })),
  );
}

const records = storedRecord(STORAGE_KEYS.exposureReviews, parse, encode);

/**
 * The stored reviews, most recent first. The value is reused while the
 * stored text is unchanged, so React can compare snapshots by identity.
 */
export function exposureReviews(): readonly ExposureReview[] {
  return records.read();
}

export function subscribeExposureReviews(listener: () => void): () => void {
  return records.subscribe(listener);
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
  return records.write(change(exposureReviews()));
}

function sameEntry(
  left: Pick<ExposureEntry, 'store' | 'dirent'>,
  right: Pick<ExposureEntry, 'store' | 'dirent'>,
): boolean {
  return left.store === right.store && left.dirent === right.dirent;
}

/**
 * Keeps a review of `entries` for the party, after its access ended. A
 * review already kept for the same party and scope is merged: an entry in
 * the new list replaces the kept one, taking its version and dropping its
 * mark, since the party could read the item again; a kept entry absent from
 * the new list stays as it was. The review moves to the front. Nothing is
 * stored for an empty list. Returns whether the review was stored.
 */
export function recordExposure(
  review: Omit<ExposureReview, 'id' | 'at'>,
  at = Date.now(),
): boolean {
  if (!review.entries.length) return true;
  const id = reviewId(review.scope, review.party);
  return update((current) => {
    const previous = current.find((seen) => seen.id === id);
    const merged = [
      ...(previous?.entries ?? []).filter(
        (seen) => !review.entries.some((entry) => sameEntry(seen, entry)),
      ),
      ...review.entries.map((entry) => ({
        store: entry.store,
        dirent: entry.dirent,
        version: entry.version,
      })),
    ];
    return [
      { ...review, id, at, entries: merged },
      ...current.filter((seen) => seen.id !== id),
    ].slice(0, REVIEW_LIMIT);
  });
}

/**
 * Runs `write`, the change that ends the party's access, and keeps `review`
 * once it succeeds. A failure whose outcome is unknown
 * ({@link failureOutcome}) keeps the review too: the change may have been
 * applied, after which the party is no longer in the roster or the device
 * list and the sheet that computed the list cannot be opened for it again.
 * A review of a change that was not applied is one the user can clear.
 * `onUnstored` runs when storage refuses the review.
 */
export async function withExposureReview<T>(
  review: Omit<ExposureReview, 'id' | 'at'>,
  write: () => Promise<T>,
  onUnstored: () => void,
): Promise<T> {
  let value: T;
  try {
    value = await write();
  } catch (error) {
    if (failureOutcome(error) === 'unknown' && !recordExposure(review))
      onUnstored();
    throw error;
  }
  if (!recordExposure(review)) onUnstored();
  return value;
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
