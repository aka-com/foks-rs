/**
 * The webview's `localStorage` records. Every key the app reads or writes is
 * registered in {@link STORAGE_KEYS}, and this module is the only one that
 * touches `localStorage`.
 *
 * Each key has a scope. A `device` record is a preference of this device,
 * whatever servers it holds. A `profile` record holds entries that name a
 * server profile, and carries the function that removes those entries:
 * {@link purgeProfiles} applies it when a profile's local state is erased by
 * Reset this device or Remove server and credentials, as the native
 * `reset_server` and `remove_server_and_credentials` commands do for the
 * profile's chat-local state.
 *
 * Storage is unavailable in restricted contexts and refuses a write when it
 * is full. {@link readStorage} and {@link writeStorage} report that as an
 * unset value and a refused write. The `OrThrow` variants are for records
 * whose caller must not proceed when the record cannot be read or stored.
 */

export interface DeviceStorageKey {
  readonly name: string;
  readonly scope: 'device';
}

export interface ProfileStorageKey {
  readonly name: string;
  readonly scope: 'profile';
  /**
   * The stored text without the entries that name one of `profiles`, or
   * `null` when nothing remains. Throws when the text cannot be read.
   */
  readonly forget: (
    raw: string,
    profiles: ReadonlySet<string>,
  ) => string | null;
}

export type StorageKey = DeviceStorageKey | ProfileStorageKey;

type JsonObject = Record<string, unknown>;

function object(value: unknown): JsonObject | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as JsonObject)
    : null;
}

function parsedObject(raw: string): JsonObject {
  const parsed = object(JSON.parse(raw));
  if (!parsed) throw new Error('Not a stored object.');
  return parsed;
}

function parsedArray(raw: string): unknown[] {
  const parsed: unknown = JSON.parse(raw);
  if (!Array.isArray(parsed)) throw new Error('Not a stored array.');
  return parsed;
}

/**
 * The profile a store ref names. A store ref from the native bridge is the
 * JSON object `store_id` writes in `src-tauri/src/commands/vault.rs`; the
 * mock bridge's refs (`acct:personal`) name no profile.
 */
function storeRefProfile(ref: unknown): string | null {
  if (typeof ref !== 'string') return null;
  try {
    const profile = object(JSON.parse(ref))?.profile;
    return typeof profile === 'string' ? profile : null;
  } catch {
    return null;
  }
}

/** The profile an encoded first-run checkpoint is bound to, if any. */
function checkpointProfile(checkpoint: unknown): string | null {
  const profile = object(object(checkpoint)?.profile)?.profile;
  return typeof profile === 'string' ? profile : null;
}

function named(profile: string | null, profiles: ReadonlySet<string>) {
  return profile !== null && profiles.has(profile);
}

const device = (name: string): DeviceStorageKey => ({ name, scope: 'device' });

export const STORAGE_KEYS = {
  /** `light`, `dark` or `system`; also read by `public/theme-init.js`. */
  appearance: device('appearance'),
  railColor: device('railColor'),
  sideCollapsed: device('sideCollapsed'),
  sidebarWidth: device('sidebarWidth'),
  filesSidebarWidth: device('filesSidebarWidth'),
  chatSidebarWidth: device('chatSidebarWidth'),
  filesView: device('filesView'),
  dismissedAddPasswordTip: device('onboarding.dismissed.add-password'),
  dismissedSetupChecklistTip: device('onboarding.dismissed.setup-checklist'),
  passwordGenerator: device('passwordGenerator'),
  trashAutoEmptyDays: device('trashAutoEmptyDays'),
  /**
   * Times keyed by a backup key's entity id, which names a key on a server
   * rather than a local profile.
   */
  recoveryPhraseTests: device('foks.recoveryPhraseTests'),
  /** The current setup checkpoint, bound to a profile once one is chosen. */
  firstRun: {
    name: 'foks.first-run.v2',
    scope: 'profile',
    forget: (raw, profiles) =>
      named(checkpointProfile(parsedObject(raw)), profiles) ? null : raw,
  },
  /** Setup attempts kept for reconciliation, each with its checkpoint. */
  setupRecovery: {
    name: 'foks.setup-recovery.v1',
    scope: 'profile',
    forget: (raw, profiles) => {
      const entries = parsedArray(raw);
      const kept = entries.filter(
        (entry) =>
          !named(checkpointProfile(object(entry)?.checkpoint), profiles),
      );
      if (kept.length === entries.length) return raw;
      return kept.length ? JSON.stringify(kept) : null;
    },
  },
  /** Invitee labels keyed `${profile}/${account}/${operationId}`. */
  invitationLabels: {
    name: 'foks.invitationLabels',
    scope: 'profile',
    forget: (raw, profiles) => {
      const labels = parsedObject(raw);
      // Profile names never contain '/'.
      const kept = Object.entries(labels).filter(([key]) => {
        const slash = key.indexOf('/');
        return slash < 0 || !profiles.has(key.slice(0, slash));
      });
      if (kept.length === Object.keys(labels).length) return raw;
      return kept.length ? JSON.stringify(Object.fromEntries(kept)) : null;
    },
  },
  /** `{ recent, favorites }`, each a list of `{ store, dirent }`. */
  recentFavorites: {
    name: 'foks.recentFavorites',
    scope: 'profile',
    forget: (raw, profiles) => {
      const lists = parsedObject(raw);
      let changed = false;
      let remaining = 0;
      const next: JsonObject = { ...lists };
      for (const list of ['recent', 'favorites']) {
        const entries = lists[list];
        if (!Array.isArray(entries)) continue;
        const kept = entries.filter(
          (entry) => !named(storeRefProfile(object(entry)?.store), profiles),
        );
        remaining += kept.length;
        if (kept.length === entries.length) continue;
        next[list] = kept;
        changed = true;
      }
      if (!changed) return raw;
      return remaining ? JSON.stringify(next) : null;
    },
  },
  /**
   * Reviews, each with the store ref of its `scope`; its entries are in
   * stores of the same profile.
   */
  exposureReviews: {
    name: 'foks.exposureReviews',
    scope: 'profile',
    forget: (raw, profiles) => {
      const reviews = parsedArray(raw);
      const kept = reviews.filter(
        (review) => !named(storeRefProfile(object(review)?.scope), profiles),
      );
      if (kept.length === reviews.length) return raw;
      return kept.length ? JSON.stringify(kept) : null;
    },
  },
} as const satisfies Record<string, StorageKey>;

const listeners = new Map<string, Set<() => void>>();

function notify(key: StorageKey): void {
  for (const listener of listeners.get(key.name) ?? []) listener();
}

/**
 * Calls `listener` after each write of `key` made through this module,
 * including writes that {@link writeStorage} reports as refused and those of
 * {@link purgeProfiles}. A write that {@link writeStorageOrThrow} refuses
 * throws to its caller without notifying.
 */
export function subscribeStorage(
  key: StorageKey,
  listener: () => void,
): () => void {
  let set = listeners.get(key.name);
  if (!set) {
    set = new Set();
    listeners.set(key.name, set);
  }
  set.add(listener);
  return () => {
    set.delete(listener);
  };
}

/** The stored text; throws when storage cannot be read. */
export function readStorageOrThrow(key: StorageKey): string | null {
  if (typeof window === 'undefined') return null;
  return window.localStorage.getItem(key.name);
}

/** The stored text, or `null` when it is unset or storage is unavailable. */
export function readStorage(key: StorageKey): string | null {
  try {
    return readStorageOrThrow(key);
  } catch {
    return null;
  }
}

/**
 * Stores `value`, or removes the record when it is `null`; throws when
 * storage refuses the write.
 */
export function writeStorageOrThrow(
  key: StorageKey,
  value: string | null,
): void {
  if (typeof window === 'undefined')
    throw new Error('Local storage is unavailable.');
  if (value === null) window.localStorage.removeItem(key.name);
  else window.localStorage.setItem(key.name, value);
  notify(key);
}

/** Like {@link writeStorageOrThrow}; returns whether storage accepted it. */
export function writeStorage(key: StorageKey, value: string | null): boolean {
  try {
    writeStorageOrThrow(key, value);
    return true;
  } catch {
    notify(key);
    return false;
  }
}

/** A decoded record that React can read with `useSyncExternalStore`. */
export interface StoredRecord<T> {
  /**
   * The decoded value. It is reused while the stored text is unchanged, so
   * React can compare snapshots by identity.
   */
  read(): T;
  /** Stores `value`; returns whether storage accepted it. */
  write(value: T): boolean;
  subscribe(listener: () => void): () => void;
}

export function storedRecord<T>(
  key: StorageKey,
  decode: (raw: string | null) => T,
  encode: (value: T) => string,
): StoredRecord<T> {
  let cached: { raw: string | null; value: T } | undefined;
  return {
    read() {
      const raw = readStorage(key);
      if (!cached || cached.raw !== raw) cached = { raw, value: decode(raw) };
      return cached.value;
    },
    write: (value) => writeStorage(key, encode(value)),
    subscribe: (listener) => subscribeStorage(key, listener),
  };
}

/**
 * Removes the entries of `profiles` from every profile-scoped record. A
 * record that cannot be parsed is removed whole: its owner reads it as empty
 * or as an error, and it may hold entries of these profiles. A record that
 * storage refuses to rewrite is left as it was.
 */
export function purgeProfiles(profiles: readonly string[]): void {
  const names = new Set(profiles);
  if (!names.size) return;
  for (const key of Object.values(STORAGE_KEYS) as StorageKey[]) {
    if (key.scope !== 'profile') continue;
    const raw = readStorage(key);
    if (raw === null) continue;
    let next: string | null;
    try {
      next = key.forget(raw, names);
    } catch {
      next = null;
    }
    if (next !== raw) writeStorage(key, next);
  }
}
