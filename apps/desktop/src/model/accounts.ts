/**
 * What the catalog says about the accounts on this Mac.
 *
 * People and Devices are both per account, and both address one by its exact
 * StoreRef, so the facts a row and a sheet title need are read here rather
 * than re-derived on each screen.
 */

import {
  serverAvailability,
  storeAvailability,
  storeDescription,
} from './lease';
import { serverDisplayName } from './types';
import type { AccountStore, AgentSnapshot } from './types';

/** The account stores on this Mac, in catalog order. */
export function accountStores(snapshot: AgentSnapshot): AccountStore[] {
  return snapshot.stores.filter(
    (store): store is AccountStore => store.kind === 'account',
  );
}

/** The username the server knows this account by, if the catalog has it. */
export function usernameOf(
  snapshot: AgentSnapshot,
  store: AccountStore,
): string | undefined {
  return snapshot.accounts.find((entry) => entry.store === store.id)?.username;
}

/** Human-readable server name, with profile disambiguation for duplicate labels. */
export function serverName(
  snapshot: AgentSnapshot,
  store: Pick<AccountStore, 'server'>,
): string {
  const server = snapshot.servers.find((entry) => entry.id === store.server);
  if (!server) return store.server;
  const displayName = serverDisplayName(server);
  const duplicate =
    server.label !== null &&
    snapshot.servers.some(
      (candidate) =>
        candidate.id !== server.id &&
        serverDisplayName(candidate) === displayName,
    );
  return duplicate ? `${displayName} · ${server.name}` : displayName;
}
/**
 * The line under a sheet title: who the workflow acts as, and where. The
 * title itself never carries the account alias.
 */
export function accountSubtitle(
  snapshot: AgentSnapshot,
  store: AccountStore,
): string {
  return `${usernameOf(snapshot, store) ?? store.account} on ${serverName(
    snapshot,
    store,
  )}`;
}

/**
 * Whether this account can be read and changed, and why not when it cannot.
 * An account is stopped by its own state or by its server's.
 */
export function accountStopped(
  snapshot: AgentSnapshot,
  store: AccountStore,
): { stopped: boolean; reason: string } {
  const server = snapshot.servers.find((entry) => entry.id === store.server);
  const serverState = server
    ? serverAvailability(snapshot, server)
    : { available: false as const };
  const stopped =
    !storeAvailability(snapshot, store).available || !serverState.available;
  return {
    stopped,
    reason: stopped
      ? `Account access is stopped · ${storeDescription(snapshot, store)}`
      : '',
  };
}
