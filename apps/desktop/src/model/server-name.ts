import type { AccountStore, AgentSnapshot } from './types';
import { serverDisplayName } from './types';

/** Human-readable server name, with profile disambiguation for duplicate labels. */
export function serverName(
  snapshot: AgentSnapshot,
  store: Pick<AccountStore, 'server'>,
): string {
  const server = snapshot.servers.find((entry) => entry.id === store.server);
  if (!server) return store.server;
  const displayName = serverDisplayName(server);
  const duplicate = snapshot.servers.some(
    (candidate) =>
      candidate.id !== server.id &&
      serverDisplayName(candidate) === displayName,
  );
  return duplicate ? `${displayName} · ${server.name}` : displayName;
}
