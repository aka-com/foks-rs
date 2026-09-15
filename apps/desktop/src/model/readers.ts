/**
 * Computes readable access for vault items based on member roles and rosters.
 *
 * Account stores return `null` as they are private to the account holder and
 * do not have member rosters.
 */

import { admits, parseRole, roleRank, visibilityOf } from './roles';
import type { Item, Party, Store, StoreRef, AgentSnapshot } from './types';

/** The store with this id, or `undefined`. */
export function storeOf(
  snapshot: AgentSnapshot,
  ref: StoreRef,
): Store | undefined {
  return snapshot.stores.find((store) => store.id === ref);
}

/** Returns all member parties belonging to the specified store. */
export function partiesOf(snapshot: AgentSnapshot, ref: StoreRef): Party[] {
  return snapshot.parties.filter((party) => party.store === ref);
}

/** Returns whether a member party can be modified or removed by the current user. */
export function actionableGroupMember(
  snapshot: AgentSnapshot,
  party: Party,
): boolean {
  return Boolean(
    party.username &&
    party.party_kind === 'user' &&
    party.locally_manageable &&
    party.label !== 'you' &&
    partiesOf(snapshot, party.store).filter(
      (candidate) => candidate.username === party.username,
    ).length === 1,
  );
}

/**
 * Selects the least-privileged actionable member in a group, prioritizing
 * lower role rank and lower visibility bands, while preserving roster order on ties.
 */
export function safestRemovalTarget(
  snapshot: AgentSnapshot,
  ref: StoreRef,
): Party | undefined {
  let safest: Party | undefined;
  let safestRank = Number.POSITIVE_INFINITY;
  let safestVisibility = Number.POSITIVE_INFINITY;
  for (const party of partiesOf(snapshot, ref)) {
    if (!actionableGroupMember(snapshot, party)) continue;
    const role = parseRole(party.destination_role);
    if (!role) continue;
    const rank = roleRank(role);
    const visibility = role.kind === 'member' ? visibilityOf(role) : 0;
    if (
      rank < safestRank ||
      (rank === safestRank && visibility < safestVisibility)
    ) {
      safest = party;
      safestRank = rank;
      safestVisibility = visibility;
    }
  }
  return safest;
}

/**
 * Determines whether a party has an active admission in the specified store.
 * User parties are always active. Federated team parties require an active
 * federation admission record.
 */
export function admissionActive(
  snapshot: AgentSnapshot,
  party: Party,
  ref: StoreRef,
): boolean {
  if (party.party_kind === 'user') return true;
  // When a party is host-scoped, the admission must match the specific remote host ID.
  const entries = snapshot.federation.filter(
    (f) =>
      f.store === ref &&
      f.remote_team_id_hex === party.party_id_hex &&
      (!party.scoped_host_id_hex ||
        f.remote_host_id_hex === party.scoped_host_id_hex),
  );
  return entries.length === 1 && entries[0]?.active === true;
}

/**
 * The roster filtered by the item's read role.
 *
 * `null` on an account store — there is no roster to filter.
 */
export function readersOf(snapshot: AgentSnapshot, item: Item): Party[] | null {
  const store = storeOf(snapshot, item.store);
  if (!store || store.kind !== 'team') return null;
  return partiesOf(snapshot, store.id).filter(
    (party) =>
      admissionActive(snapshot, party, store.id) &&
      admits(party.destination_role, item.read),
  );
}

/** A party's display name: username, team name, else a truncated id. */
export function partyName(party: Party): string {
  return (
    party.username ?? party.team_name ?? `${party.party_id_hex.slice(0, 10)}…`
  );
}

/** "1 person" / "5 people". */
export function peopleLabel(count: number): string {
  return `${count} ${count === 1 ? 'person' : 'people'}`;
}

/**
 * Formats a summary string of member users and federated groups
 * (e.g., "5 people · 1 group" or "2 people").
 */
export function peopleGroups(parties: readonly Party[]): string {
  const groups = parties.filter((p) => p.party_kind !== 'user').length;
  const people = parties.length - groups;
  const head = peopleLabel(people);
  if (!groups) return head;
  return `${head} · ${groups} ${groups === 1 ? 'group' : 'groups'}`;
}
