/**
 * What the catalog says about the accounts on this Mac.
 *
 * Account and Devices are both per account, and both address one by its exact
 * StoreRef, so the facts a row and a sheet title need are read here rather
 * than re-derived on each screen.
 */

import { storeOperationAvailability, storeDescription } from './lease';
import { serverDisplayLabelForStore } from './server-name';
export { serverDisplayLabelForStore } from './server-name';
import type { AccountStore, AgentSnapshot } from './types';
import type { StoreOperation } from './lease';

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

/**
 * The line under a sheet title: who the workflow acts as, and where. The
 * title itself never carries the account alias.
 */
export function accountSubtitle(
  snapshot: AgentSnapshot,
  store: AccountStore,
): string {
  return `${usernameOf(snapshot, store) ?? store.account} on ${serverDisplayLabelForStore(
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
  operation: StoreOperation = 'metadata',
): { stopped: boolean; reason: string } {
  const stopped = !storeOperationAvailability(snapshot, store, operation)
    .available;
  return {
    stopped,
    reason: stopped
      ? `Account access is stopped · ${storeDescription(snapshot, store, { operation })}`
      : '',
  };
}

/** Display-only; never pass this value to a command or use it as a StoreRef. */
export function localAliasOf(
  snapshot: AgentSnapshot,
  store: AccountStore,
): string {
  return (
    snapshot.accounts.find((entry) => entry.store === store.id)?.localAlias ??
    store.account
  );
}
