import type { Store, AgentSnapshot } from './types';

/**
 * Sorts stores in display order: grouped by configured server order,
 * with account stores preceding team stores.
 */
export function storeDisplayOrder(snapshot: AgentSnapshot): Store[] {
  const servers = new Map(
    snapshot.servers.map((server, index) => [server.id, index]),
  );
  const source = new Map(
    snapshot.stores.map((store, index) => [store.id, index]),
  );
  return [...snapshot.stores].sort(
    (left, right) =>
      (servers.get(left.server) ?? Number.MAX_SAFE_INTEGER) -
        (servers.get(right.server) ?? Number.MAX_SAFE_INTEGER) ||
      (left.kind === right.kind ? 0 : left.kind === 'account' ? -1 : 1) ||
      (source.get(left.id) ?? 0) - (source.get(right.id) ?? 0),
  );
}

export function storeNavigationOrder(snapshot: AgentSnapshot): Store[] {
  return [
    ...snapshot.stores.filter((store) => store.kind === 'account'),
    ...snapshot.stores.filter(
      (store) => store.kind === 'team' && store.team_kind === 'named',
    ),
    ...snapshot.stores.filter(
      (store) => store.kind === 'team' && store.team_kind === 'adhoc',
    ),
  ];
}
