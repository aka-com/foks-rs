/**
 * The rules the Teams list, the group page and the sheets all decide by.
 *
 * One predicate pair and one set of sentences, so a row's menu and the page it
 * opens never disagree about what applies or why. Nothing here renders.
 */

import {
  groupDetailFailure,
  partiesOf,
  roleRank,
  serverDisplayLabel,
  serverDisplayLabelForStore,
  storeOperationAvailability,
} from '../model';
import type {
  Account,
  AccountStore,
  AgentSnapshot,
  Server,
  StoreRef,
  TeamStore,
} from '../model';

/**
 * What discovery needs to run for one account, resolved from its exact
 * identity.
 *
 * `Account.store` is the identity; the alias is profile-local and two
 * profiles can each hold `personal`, so resolving by alias would sooner or
 * later authenticate the wrong account. Every relationship is re-checked
 * here and any inconsistency fails closed, because the caller is about to
 * write durable local bindings with whatever this returns.
 */
export interface DiscoveryContext {
  account: Account;
  store: AccountStore;
  server: Server;
  /** Discovery reads the server, so a stopped server cannot be checked. */
  available: boolean;
}

export function discoveryContext(
  snapshot: AgentSnapshot,
  ref: StoreRef | null,
): DiscoveryContext | null {
  if (!ref) return null;
  const account = snapshot.accounts.find(
    (candidate) => candidate.store === ref,
  );
  if (!account) return null;
  const store = snapshot.stores.find(
    (candidate) => candidate.id === account.store,
  );
  if (!store || store.kind !== 'account') return null;
  if (account.alias !== store.account || account.server !== store.server)
    return null;
  const server = snapshot.servers.find(
    (candidate) => candidate.profileName === store.server,
  );
  if (!server) return null;
  return {
    account,
    store,
    server,
    available: storeOperationAvailability(snapshot, store, 'teams').available,
  };
}

/** One accessible name per button, since several read "Check for teams". */
export const checkLabel = (context: DiscoveryContext): string =>
  `Check for teams accessible to ${context.account.username} on ${serverDisplayLabel(context.server)}`;

export const unavailableTitle = (context: DiscoveryContext): string =>
  `Restore access to ${serverDisplayLabel(context.server)} before checking for teams.`;

/**
 * Why an invitation cannot be written: the message names the server the
 * invitee joins, so it cannot be composed while that server is out of reach.
 */
export const inviteUnavailableTitle = (serverName: string): string =>
  `Restore access to ${serverName} before inviting someone.`;

/**
 * Why this Mac cannot change a group's roster, or its federated team members —
 * `undefined` when it can. One rule for both the Teams list and the group page,
 * so a row's menu and the page it opens never disagree about what applies.
 */
export function manageReason(
  snapshot: AgentSnapshot,
  store: TeamStore,
  source: 'roster' | 'federation',
): string | undefined {
  if (store.team_kind !== 'named')
    return 'Memberships can’t be changed in an ad-hoc team.';
  if (store.active === false) return 'Finish setting up this team first.';
  if (
    !storeOperationAvailability(
      snapshot,
      store,
      source === 'roster' ? 'teams' : 'federation',
    ).available
  )
    return `Restore access to ${serverDisplayLabelForStore(snapshot, store)} first.`;
  if (groupDetailFailure(snapshot, store.id, source))
    return source === 'roster'
      ? 'The roster could not be read. Refresh before making changes.'
      : 'The federated teams could not be read. Refresh before making changes.';
  // The role this Mac holds is a roster fact, so an unread roster is not
  // evidence that it lacks one: adding a federated team is refused for what failed to
  // load, not for a permission nothing could have checked.
  if (groupDetailFailure(snapshot, store.id, 'roster'))
    return 'The roster could not be read. Refresh before making changes.';
  const mine = partiesOf(snapshot, store.id).find(
    (party) => party.label === 'you',
  );
  return mine && roleRank(mine.destination_role) >= 2
    ? undefined
    : 'Only an Admin or an Owner can change this team’s members.';
}

/** Whether this Mac can add to or change the group's roster. */
export function rosterManageable(
  snapshot: AgentSnapshot,
  store: TeamStore,
): boolean {
  return manageReason(snapshot, store, 'roster') === undefined;
}

/** Whether this Mac can add another federated team here, or remove one. */
export function federationManageable(
  snapshot: AgentSnapshot,
  store: TeamStore,
): boolean {
  return manageReason(snapshot, store, 'federation') === undefined;
}
