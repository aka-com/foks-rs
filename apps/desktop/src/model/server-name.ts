import type { AccountStore, AgentSnapshot } from './types';
import { serverDisplayLabel } from './types';

/** Human-readable server label, with profile disambiguation for duplicates. */
export function serverDisplayLabelForStore(
  snapshot: AgentSnapshot,
  store: Pick<AccountStore, 'server'>,
): string {
  const server = snapshot.servers.find(
    (entry) => entry.profileName === store.server,
  );
  if (!server) return store.server;
  const displayName = serverDisplayLabel(server);
  const duplicate = snapshot.servers.some(
    (candidate) =>
      candidate.profileName !== server.profileName &&
      serverDisplayLabel(candidate) === displayName,
  );
  return duplicate ? `${displayName} · ${server.profileName}` : displayName;
}
