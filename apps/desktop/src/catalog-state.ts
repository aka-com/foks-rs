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

export function markCatalogRefresh(
  snapshot: AgentSnapshot,
  profiles: readonly string[] = snapshot.catalogProfiles,
  nowSeconds = Math.floor(Date.now() / 1000),
): AgentSnapshot {
  return updateAttempt(snapshot, profiles, nowSeconds);
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
): AgentSnapshot {
  const freshness = snapshot.catalogFreshness;
  const update = (previous?: CatalogFreshnessEntry): CatalogFreshnessEntry => ({
    ...previous,
    lastAttemptAt: nowSeconds,
    refreshing: !error,
    error,
  });
  return {
    ...snapshot,
    catalogFreshness: {
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
    error,
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
        catalogItemsComplete(response, store.server, partial) &&
        inventory?.status === 'available';
      return [
        store.id,
        entry(
          previous?.stores[store.id],
          complete,
          failure?.error,
        ),
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
  return { profiles, stores };
}

export function mergeProfileSnapshot(
  base: AgentSnapshot,
  projected: AgentSnapshot,
  profile: string,
): AgentSnapshot {
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
      ...base.observedExpiredLeases.filter((entry) => entry.profile !== profile),
      ...projected.observedExpiredLeases,
    ],
    notifications: [
      ...base.notifications.filter(
        (entry) =>
          entry.profile !== profile && !replacedNotifications.has(entry.id),
      ),
      ...projected.notifications,
    ],
    catalogFreshness: {
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
