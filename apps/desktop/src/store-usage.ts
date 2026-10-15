/**
 * How much a vault stores on its server, read when a view that shows it
 * opens. The server computes usage by decoding the vault's nodes on every
 * request, so a figure is kept for the session and read again only after the
 * vault's listed files change, on an explicit Refresh, or after access is
 * reset; it is never re-read or retried on the metadata cadence.
 *
 * The figure is the server's KV usage: the vault's file count and the files'
 * total ciphertext bytes. The server reports no limit with it, a Go host sets
 * its own, and the quota a write is refused against counts more than file
 * ciphertext (directories, metadata, envelopes), so nothing here states a
 * fraction of a quota or refuses a write in advance.
 */
import { useEffect, useMemo } from 'react';
import { enqueueProfileWork } from './bridge';
import type { Bridge, StoreUsage } from './bridge';
import { itemsReadiness } from './model';
import type { AgentSnapshot, Store } from './model';
import { useMetadataQuery, useMetadataRepository } from './data/query-hooks';
import type {
  MetadataQuery,
  MetadataRepository,
  QueryKey,
  QuerySnapshot,
} from './data/metadata-repository';

export const storeUsageKey = (store: Store) =>
  ['store-usage', store.server, store.id] as const;

/** Usage rows are read on demand only; the metadata cadence skips them. */
export const isStoreUsageQuery = (key: QueryKey): boolean =>
  key[0] === 'store-usage';

export function storeUsageQuery(
  repository: MetadataRepository,
  bridge: Bridge,
  store: Store,
): MetadataQuery<StoreUsage> {
  const { server, id } = store;
  return repository.query<StoreUsage>(
    storeUsageKey(store),
    () =>
      enqueueProfileWork(
        bridge,
        server,
        () => bridge.storeUsage(id),
        'store-usage',
      ),
    // Current until invalidated: a change to the vault's files, an explicit
    // Refresh, an access reset or a profile replacement. Time alone does not
    // age it.
    Number.POSITIVE_INFINITY,
  );
}

/**
 * A digest of the vault's listed files, or `undefined` while the listing is
 * not loaded. It changes when a file is added, removed, moved or rewritten,
 * which is when the stored total can change.
 */
export function storeContentVersion(
  snapshot: AgentSnapshot,
  store: Store,
): string | undefined {
  if (itemsReadiness(snapshot, store.id) !== 'ready') return undefined;
  // FNV-1a over each file's path and version. Collisions only delay a
  // re-read until the next change.
  let hash = 0x811c9dc5;
  let files = 0;
  for (const item of snapshot.items) {
    if (item.store !== store.id || item.kind === 'Folder') continue;
    files++;
    for (const char of `${item.path}\0${item.version}\n`) {
      hash ^= char.codePointAt(0) ?? 0;
      hash = Math.imul(hash, 0x01000193) >>> 0;
    }
  }
  return `${files}:${hash.toString(16)}`;
}

/** The listing digest each usage row was last shown against. */
const shownAgainst = new WeakMap<MetadataQuery<StoreUsage>, string>();

/**
 * Reads a vault's usage while `enabled`, and again when the vault's listed
 * files have changed since the figure was last shown. A failed read is
 * reported through the returned state, not as an error notice.
 */
export function useStoreUsage(
  bridge: Bridge,
  snapshot: AgentSnapshot,
  store: Store,
  enabled: boolean,
): QuerySnapshot<StoreUsage> {
  const repository = useMetadataRepository(bridge);
  // The repository returns one row per key, so a republished store object
  // still yields the same query.
  const query = useMemo(
    () => (enabled ? storeUsageQuery(repository, bridge, store) : null),
    [repository, bridge, enabled, store],
  );
  const content = useMemo(
    () => storeContentVersion(snapshot, store),
    [snapshot, store],
  );
  // Runs before `useMetadataQuery`'s load effect, so a figure shown against
  // an older listing is replaced rather than shown and then reloaded.
  useEffect(() => {
    if (!query || content === undefined) return;
    const before = shownAgainst.get(query);
    shownAgainst.set(query, content);
    if (before !== undefined && before !== content) query.invalidate();
  }, [query, content]);
  return useMetadataQuery(query);
}

const UNITS = ['B', 'KiB', 'MiB', 'GiB', 'TiB', 'PiB'] as const;

/** A byte count in binary units: `512 B`, `3.4 KiB`, `2.3 GiB`, `14 GiB`. */
export function formatStoredBytes(bytes: number): string {
  let value = bytes;
  let unit = 0;
  while (unit < UNITS.length - 1 && value >= 1024) {
    value /= 1024;
    unit++;
  }
  if (unit === 0) return `${bytes} B`;
  let shown = value < 10 ? value.toFixed(1) : Math.round(value).toString();
  // 1023.96 KiB rounds to 1024 KiB; state it as 1.0 MiB.
  if (Number(shown) >= 1024 && unit < UNITS.length - 1) {
    unit++;
    shown = (value / 1024).toFixed(1);
  }
  return `${shown} ${UNITS[unit]}`;
}

/** `About 2.3 GiB stored (encrypted) · 1,204 files`. */
export function storeUsageSummary({ files, bytes }: StoreUsage): string {
  const size =
    bytes < 1024 ? `${bytes} B` : `About ${formatStoredBytes(bytes)}`;
  const count = `${files.toLocaleString('en-US')} ${files === 1 ? 'file' : 'files'}`;
  return `${size} stored (encrypted) · ${count}`;
}

/**
 * The usage line for a view: the figure once read (kept while a re-read is
 * under way or fails), otherwise a loading or unavailable statement.
 */
export function storeUsageText(state: QuerySnapshot<StoreUsage>): string {
  if (state.data) return storeUsageSummary(state.data);
  if (state.error !== undefined) return 'Storage used unavailable.';
  return 'Reading storage used…';
}
