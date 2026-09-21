import type { CatalogDto } from './bridge';
import type {
  AgentSnapshot,
  CatalogFreshness,
  CatalogFreshnessEntry,
  ServerFailure,
  Store,
} from './model';

export function sameStoreIdentity(a: Store, b: Store): boolean {
  return (
    a.id === b.id &&
    a.server === b.server &&
    a.kind === b.kind &&
    a.account === b.account &&
    (a.kind !== 'team' ||
      (b.kind === 'team' && a.team_id_hex === b.team_id_hex))
  );
}

export function catalogItemsComplete(
  response: CatalogDto,
  profile: string,
  partial: boolean,
): boolean {
  return !partial || response.fullItemReads?.includes(profile) === true;
}

export function catalogStoreComplete(
  response: CatalogDto,
  store: Store,
  partial: boolean,
): boolean {
  const read = response.storeReads?.find((entry) => entry.store === store.id);
  return read
    ? read.state === 'complete'
    : catalogItemsComplete(response, store.server, partial);
}

export function markCatalogRefresh(
  snapshot: AgentSnapshot,
  profiles?: readonly string[],
  nowSeconds = Math.floor(Date.now() / 1000),
): AgentSnapshot {
  const next = updateAttempt(
    snapshot,
    profiles ?? snapshot.catalogProfiles,
    nowSeconds,
    undefined,
    profiles === undefined,
  );
  if (profiles !== undefined) return next;
  return {
    ...next,
    catalogFreshness: {
      ...next.catalogFreshness!,
      attempt: {
        ...snapshot.catalogFreshness?.attempt,
        lastAttemptAt: nowSeconds,
        refreshing: true,
        error: undefined,
      },
    },
  };
}

/**
 * Clears the refreshing flag after a profile refresh is retired. Preserve the
 * previous success and error state because the retired read produced no result.
 */
export function settleCatalogRefresh(
  snapshot: AgentSnapshot,
  profiles: readonly string[],
): AgentSnapshot {
  const freshness = snapshot.catalogFreshness;
  if (!freshness) return snapshot;
  const settle = (entry: CatalogFreshnessEntry | undefined) =>
    entry?.refreshing ? { ...entry, refreshing: false } : entry;
  const settled = (
    entries: Readonly<Record<string, CatalogFreshnessEntry>>,
    keep: (key: string) => boolean,
  ) =>
    Object.fromEntries(
      Object.entries(entries).map(([key, entry]) => [
        key,
        keep(key) ? (settle(entry) ?? entry) : entry,
      ]),
    );
  const stores = new Set(
    snapshot.stores
      .filter((store) => profiles.includes(store.server))
      .map((store) => store.id),
  );
  return {
    ...snapshot,
    catalogFreshness: {
      ...freshness,
      profiles: settled(freshness.profiles, (key) => profiles.includes(key)),
      stores: settled(freshness.stores, (key) => stores.has(key)),
    },
  };
}

export function failWholeCatalogRefresh(
  snapshot: AgentSnapshot,
  error: ServerFailure,
  nowSeconds = Math.floor(Date.now() / 1000),
): AgentSnapshot {
  const freshness = snapshot.catalogFreshness;
  const settled = (entries: CatalogFreshness['profiles'] = {}) =>
    Object.fromEntries(
      Object.entries(entries).map(([key, entry]) => [
        key,
        entry.refreshing ? { ...entry, refreshing: false } : entry,
      ]),
    );
  return {
    ...snapshot,
    catalogFreshness: {
      attempt: {
        ...freshness?.attempt,
        lastAttemptAt: nowSeconds,
        refreshing: false,
        error,
      },
      profiles: settled(freshness?.profiles),
      stores: settled(freshness?.stores),
    },
  };
}

export function failCatalogRefresh(
  snapshot: AgentSnapshot,
  profiles: readonly string[],
  error: ServerFailure,
  nowSeconds = Math.floor(Date.now() / 1000),
): AgentSnapshot {
  return updateAttempt(snapshot, profiles, nowSeconds, error);
}

function updateAttempt(
  snapshot: AgentSnapshot,
  profiles: readonly string[],
  nowSeconds: number,
  error?: ServerFailure,
  preserveError = false,
): AgentSnapshot {
  const freshness = snapshot.catalogFreshness;
  const update = (previous?: CatalogFreshnessEntry): CatalogFreshnessEntry => ({
    ...previous,
    lastAttemptAt: nowSeconds,
    refreshing: !error,
    error: error ?? (preserveError ? previous?.error : undefined),
  });
  return {
    ...snapshot,
    catalogFreshness: {
      ...freshness,
      profiles: {
        ...freshness?.profiles,
        ...Object.fromEntries(
          profiles.map((profile) => [
            profile,
            update(freshness?.profiles[profile]),
          ]),
        ),
      },
      stores: {
        ...freshness?.stores,
        ...Object.fromEntries(
          snapshot.stores
            .filter((store) => profiles.includes(store.server))
            .map((store) => [store.id, update(freshness?.stores[store.id])]),
        ),
      },
    },
  };
}

export function projectCatalogFreshness(
  snapshot: AgentSnapshot,
  base: AgentSnapshot | undefined,
  response: CatalogDto,
  partial: boolean,
  nowSeconds: number,
  profileScope?: string,
): CatalogFreshness {
  const previous = base?.catalogFreshness;
  const entry = (
    prior: CatalogFreshnessEntry | undefined,
    complete: boolean,
    error?: ServerFailure,
  ): CatalogFreshnessEntry => ({
    ...prior,
    lastAttemptAt: nowSeconds,
    ...(complete && !error ? { lastSuccessAt: nowSeconds } : {}),
    refreshing: partial && !complete && !error,
    error: error ?? (partial && !complete ? prior?.error : undefined),
  });
  const stores = Object.fromEntries(
    snapshot.stores.map((store) => {
      const inventory = snapshot.storeInventory.find(
        (state) => state.store === store.id,
      );
      const failure = response.failures.find(
        (failure) =>
          failure.profile === store.server &&
          (failure.scope === 'profile' || failure.store === store.id),
      );
      const complete =
        catalogStoreComplete(response, store, partial) &&
        inventory?.status === 'available';
      return [
        store.id,
        entry(previous?.stores[store.id], complete, failure?.error),
      ];
    }),
  );
  const profiles = Object.fromEntries(
    snapshot.catalogProfiles.map((profile) => {
      const inventory = snapshot.profileInventory.find(
        (state) => state.profile === profile,
      );
      const server = snapshot.servers.find((server) => server.id === profile);
      const failure = response.failures.find(
        (failure) => failure.profile === profile,
      );
      const error =
        failure?.error ??
        (server?.passiveStatus.status === 'failed'
          ? server.passiveStatus.error
          : undefined);
      const complete =
        catalogItemsComplete(response, profile, partial) &&
        (response.fullItemReads === undefined ||
          response.fullItemReads.includes(profile)) &&
        inventory?.accounts === 'complete' &&
        inventory.teams === 'complete' &&
        server?.passiveStatus.status === 'available' &&
        snapshot.stores
          .filter((store) => store.server === profile)
          .every((store) =>
            snapshot.storeInventory.some(
              (state) =>
                state.store === store.id && state.status === 'available',
            ),
          );
      return [profile, entry(previous?.profiles[profile], complete, error)];
    }),
  );
  return {
    ...(profileScope === undefined
      ? { attempt: entry(previous?.attempt, !partial) }
      : {}),
    profiles,
    stores,
  };
}

export function mergeProfileSnapshot(
  base: AgentSnapshot,
  projected: AgentSnapshot,
  profile: string,
): AgentSnapshot {
  const scopedStores = projected.stores.filter(
    (store) => store.server === profile,
  );
  const scopedIds = new Set(scopedStores.map((store) => store.id));
  const inScope = (entry: { store: string }) => scopedIds.has(entry.store);
  projected = {
    ...projected,
    servers: projected.servers.filter((server) => server.id === profile),
    accounts: projected.accounts.filter(
      (account) => account.server === profile,
    ),
    stores: scopedStores,
    storeInventory: projected.storeInventory.filter(inScope),
    profileInventory: projected.profileInventory.filter(
      (entry) => entry.profile === profile,
    ),
    items: projected.items.filter(inScope),
    parties: projected.parties.filter(inScope),
    federation: projected.federation.filter(inScope),
    groupDetailFailures: projected.groupDetailFailures.filter(inScope),
    observedExpiredLeases: projected.observedExpiredLeases.filter(
      (entry) => entry.profile === profile,
    ),
    notifications: projected.notifications.filter(
      (entry) => entry.profile === profile,
    ),
    catalogFreshness: {
      profiles: Object.fromEntries(
        Object.entries(projected.catalogFreshness?.profiles ?? {}).filter(
          ([key]) => key === profile,
        ),
      ),
      stores: Object.fromEntries(
        Object.entries(projected.catalogFreshness?.stores ?? {}).filter(
          ([key]) => scopedIds.has(key),
        ),
      ),
    },
  };
  const previousIds = new Set(
    base.stores
      .filter((store) => store.server === profile)
      .map((store) => store.id),
  );
  const outsideStore = (entry: { store: string }) =>
    !previousIds.has(entry.store);
  const replacedNotifications = new Set(
    projected.notifications.map((entry) => entry.id),
  );
  return {
    ...base,
    agent: projected.agent,
    servers: [
      ...base.servers.filter((entry) => entry.id !== profile),
      ...projected.servers,
    ],
    accounts: [
      ...base.accounts.filter((entry) => entry.server !== profile),
      ...projected.accounts,
    ],
    stores: [
      ...base.stores.filter((entry) => entry.server !== profile),
      ...projected.stores,
    ],
    storeInventory: [
      ...base.storeInventory.filter(outsideStore),
      ...projected.storeInventory,
    ],
    profileInventory: [
      ...base.profileInventory.filter((entry) => entry.profile !== profile),
      ...projected.profileInventory,
    ],
    items: [...base.items.filter(outsideStore), ...projected.items],
    parties: [...base.parties.filter(outsideStore), ...projected.parties],
    federation: [
      ...base.federation.filter(outsideStore),
      ...projected.federation,
    ],
    groupDetailFailures: [
      ...base.groupDetailFailures.filter(outsideStore),
      ...projected.groupDetailFailures,
    ],
    observedExpiredLeases: [
      ...new Map(
        [...base.observedExpiredLeases, ...projected.observedExpiredLeases].map(
          (entry) => [
            entry.profile,
            {
              profile: entry.profile,
              expiresAt: Math.max(
                entry.expiresAt,
                base.observedExpiredLeases.find(
                  (previous) => previous.profile === entry.profile,
                )?.expiresAt ?? 0,
              ),
            },
          ],
        ),
      ).values(),
    ],
    notifications: [
      ...base.notifications.filter(
        (entry) =>
          entry.profile !== profile && !replacedNotifications.has(entry.id),
      ),
      ...projected.notifications,
    ],
    catalogFreshness: {
      ...base.catalogFreshness,
      profiles: {
        ...base.catalogFreshness?.profiles,
        ...projected.catalogFreshness?.profiles,
      },
      stores: {
        ...Object.fromEntries(
          Object.entries(base.catalogFreshness?.stores ?? {}).filter(
            ([store]) => !previousIds.has(store),
          ),
        ),
        ...projected.catalogFreshness?.stores,
      },
    },
  };
}
