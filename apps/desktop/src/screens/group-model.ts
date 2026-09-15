/**
 * The rules the Teams list, the group page and the sheets all decide by.
 *
 * One predicate pair and one set of sentences, so a row's menu and the page it
 * opens never disagree about what applies or why. Nothing here renders.
 */

import {
  groupDetailFailure,
  partiesOf,
  partyName,
  roleRank,
  serverOf,
  storeReadable,
} from '../model';
import type {
  Account,
  AccountStore,
  AgentSnapshot,
  Server,
  Store,
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
    (candidate) => candidate.id === store.server,
  );
  if (!server) return null;
  return {
    account,
    store,
    server,
    available: storeReadable(snapshot, store.id),
  };
}

/** One accessible name per button, since several read "Check for groups". */
export const checkLabel = (context: DiscoveryContext): string =>
  `Check for groups accessible to ${context.account.username} on ${context.server.name}`;

export const unavailableTitle = (context: DiscoveryContext): string =>
  `Restore access to ${context.server.name} before checking for groups.`;

/**
 * Why an invitation cannot be written: the message names the server the
 * invitee joins, so it cannot be composed while that server is out of reach.
 */
export const inviteUnavailableTitle = (serverName: string): string =>
  `Restore access to ${serverName} before inviting someone.`;

function oxfordOr(names: readonly string[]): string {
  if (names.length <= 1) return names[0] ?? '';
  if (names.length === 2) return `${names[0]} or ${names[1]}`;
  return `${names.slice(0, -1).join(', ')}, or ${names[names.length - 1]}`;
}

/**
 * Why this Mac cannot change a group's roster, or the groups admitted into it —
 * `undefined` when it can. One rule for both the Teams list and the group page,
 * so a row's menu and the page it opens never disagree about what applies.
 */
export function manageReason(
  snapshot: AgentSnapshot,
  store: TeamStore,
  source: 'roster' | 'federation',
): string | undefined {
  if (store.team_kind !== 'named')
    return 'Memberships can’t be changed in an ad-hoc group.';
  if (store.active === false) return 'Finish setting up this group first.';
  if (!storeReadable(snapshot, store.id))
    return `Restore access to ${serverOf(snapshot, store.id)?.name ?? store.server} first.`;
  if (groupDetailFailure(snapshot, store.id, source))
    return source === 'roster'
      ? 'The roster could not be read. Refresh before making changes.'
      : 'The admitted groups could not be read. Refresh before making changes.';
  // The role this Mac holds is a roster fact, so an unread roster is not
  // evidence that it lacks one: an admission is refused for what failed to
  // load, not for a permission nothing could have checked.
  if (groupDetailFailure(snapshot, store.id, 'roster'))
    return 'The roster could not be read. Refresh before making changes.';
  const mine = partiesOf(snapshot, store.id).find(
    (party) => party.label === 'you',
  );
  return mine && roleRank(mine.destination_role) >= 2
    ? undefined
    : 'Only an Admin or an Owner can change this group’s members.';
}

/** Whether this Mac can add to or change the group's roster. */
export function rosterManageable(
  snapshot: AgentSnapshot,
  store: TeamStore,
): boolean {
  return manageReason(snapshot, store, 'roster') === undefined;
}

/** Whether this Mac can admit another group here, or drop one. */
export function federationManageable(
  snapshot: AgentSnapshot,
  store: TeamStore,
): boolean {
  return manageReason(snapshot, store, 'federation') === undefined;
}

/**
 * Why leaving is unavailable: there is no leave command to offer. Which of the
 * two sentences applies can only be told from a roster that loaded — an empty
 * or unread roster is not evidence of sole ownership.
 */
export function leaveReason(snapshot: AgentSnapshot, store: Store): string {
  const roster = partiesOf(snapshot, store.id);
  if (!roster.length || groupDetailFailure(snapshot, store.id, 'roster'))
    return 'Leaving a group is not available yet.';
  const seniors = roster
    .filter(
      (party) =>
        party.party_kind === 'user' &&
        party.label !== 'you' &&
        roleRank(party.destination_role) >= 2,
    )
    .map((party) => partyName(party));
  return seniors.length
    ? `To leave this group, ask ${oxfordOr(seniors)} to remove your account.`
    : 'As the sole owner, you must transfer ownership or delete the group to leave.';
}
