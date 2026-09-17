import type { Store, AgentSnapshot } from './types';

/**
 * A team whose creation never finished. It holds no members, channels or
 * items and refuses every action but finishing or removing it, so it is
 * listed after the stores that work rather than among them.
 */
export function creationInterrupted(store: Store): boolean {
  return store.kind === 'team' && store.active === false;
}

/**
 * The same list with interrupted creations moved to its end, each of the two
 * runs keeping the order it already had.
 */
function interruptedLast<T extends Store>(stores: readonly T[]): T[] {
  return [
    ...stores.filter((store) => !creationInterrupted(store)),
    ...stores.filter(creationInterrupted),
  ];
}

/**
 * Sorts stores in display order: grouped by configured server order,
 * with account stores preceding team stores, and a team whose creation never
 * finished after the teams that work on its own server.
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
      Number(creationInterrupted(left)) - Number(creationInterrupted(right)) ||
      (source.get(left.id) ?? 0) - (source.get(right.id) ?? 0),
  );
}

/**
 * Accounts, then named teams, then ad-hoc shares. Within each run of teams a
 * creation that never finished comes last, and the rest keep the order the
 * snapshot gave them.
 */
export function storeNavigationOrder(snapshot: AgentSnapshot): Store[] {
  return [
    ...snapshot.stores.filter((store) => store.kind === 'account'),
    ...interruptedLast(
      snapshot.stores.filter(
        (store) => store.kind === 'team' && store.team_kind === 'named',
      ),
    ),
    ...interruptedLast(
      snapshot.stores.filter(
        (store) => store.kind === 'team' && store.team_kind === 'adhoc',
      ),
    ),
  ];
}
