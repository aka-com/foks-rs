import type { AgentSnapshot, CollectionReadiness, StoreRef } from './types';

export function combineReadiness(
  states: readonly CollectionReadiness[],
): CollectionReadiness {
  return states.includes('loading')
    ? 'loading'
    : states.includes('unavailable')
      ? 'unavailable'
      : 'ready';
}

/** Unknown inventory is loading only while a read can still supply it. */
function unfinished(
  snapshot: AgentSnapshot,
  profile?: string,
): CollectionReadiness {
  const scoped = profile
    ? snapshot.catalogFreshness?.profiles[profile]
    : undefined;
  const attempt = snapshot.catalogFreshness?.attempt;
  if (scoped?.refreshing || attempt?.refreshing) return 'loading';
  if (scoped?.error || attempt?.error || snapshot.catalogFreshness)
    return 'unavailable';
  return 'loading';
}

export function inventoryReadiness(
  snapshot: AgentSnapshot,
  kind: 'profiles' | 'accounts' | 'teams',
  profile?: string,
): CollectionReadiness {
  if (snapshot.profileInventoryStatus !== 'complete')
    return unfinished(snapshot, profile);
  if (kind === 'profiles') return 'ready';
  const profiles = profile ? [profile] : snapshot.catalogProfiles;
  return combineReadiness(
    profiles.map((id) =>
      snapshot.profileInventory.find((entry) => entry.profile === id)?.[
        kind
      ] === 'complete'
        ? 'ready'
        : unfinished(snapshot, id),
    ),
  );
}

export function groupDetailReadiness(
  snapshot: AgentSnapshot,
  store: StoreRef,
  source: 'roster' | 'federation',
): CollectionReadiness {
  if (
    snapshot.groupDetailFailures.some(
      (failure) => failure.store === store && failure.source === source,
    )
  )
    return 'unavailable';
  const state =
    snapshot.groupDetailInventory?.find((entry) => entry.store === store)?.[
      source
    ] ?? 'ready';
  if (state !== 'loading') return state;
  // Roster enrichment follows the item catalog, so a profile can have finished
  // its catalog read while these details are still explicitly pending.
  const profile = snapshot.stores.find((entry) => entry.id === store)?.server;
  const scoped = profile
    ? snapshot.catalogFreshness?.profiles[profile]
    : undefined;
  const attempt = snapshot.catalogFreshness?.attempt;
  return (scoped?.error && !scoped.refreshing) ||
    (attempt?.error && !attempt.refreshing)
    ? 'unavailable'
    : state;
}

export function itemsReadiness(
  snapshot: AgentSnapshot,
  store?: StoreRef,
): CollectionReadiness {
  const selected = store
    ? snapshot.stores.filter((entry) => entry.id === store)
    : snapshot.stores;
  const inventories = new Map(
    snapshot.storeInventory.map((entry) => [entry.store, entry]),
  );
  return combineReadiness([
    ...(store
      ? []
      : [
          inventoryReadiness(snapshot, 'accounts'),
          inventoryReadiness(snapshot, 'teams'),
        ]),
    ...(store && selected.length === 0 ? ['unavailable' as const] : []),
    ...selected.map((entry): CollectionReadiness => {
      const inventory = inventories.get(entry.id);
      return inventory?.status === 'available'
        ? 'ready'
        : inventory?.status === 'loading'
          ? unfinished(snapshot, entry.server)
          : 'unavailable';
    }),
  ]);
}
