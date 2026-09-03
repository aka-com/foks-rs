/**
 * Who can read an item — ported from `wave6/shell.js:217-241`.
 *
 * "Readable by N" is always this computation (read role × roster), never
 * prose. An account store has no roster and nobody else in it, so it answers
 * `null` rather than a count: sharing anything means putting it in a team.
 */

import { admits, parseRole, roleRank, visibilityOf } from './roles';
import type { Item, Party, Store, StoreRef, World } from './types';

/** The store with this id, or `undefined`. */
export function storeOf(world: World, ref: StoreRef): Store | undefined {
  return world.stores.find((store) => store.id === ref);
}

/** Every party on a store, in fixture order. */
export function partiesOf(world: World, ref: StoreRef): Party[] {
  return world.parties.filter((party) => party.store === ref);
}

/** A roster row the desktop can name safely in a member mutation. */
export function actionableGroupMember(world: World, party: Party): boolean {
  return Boolean(
    party.username &&
    party.party_kind === 'user' &&
    party.locally_manageable &&
    party.label !== 'you' &&
    partiesOf(world, party.store).filter(
      (candidate) => candidate.username === party.username,
    ).length === 1,
  );
}

/**
 * The least-authoritative actionable member, preserving roster order on ties.
 * Member visibility is live authority too: a lower band is safer than a
 * higher band before considering the next roster row.
 */
export function safestRemovalTarget(
  world: World,
  ref: StoreRef,
): Party | undefined {
  let safest: Party | undefined;
  let safestRank = Number.POSITIVE_INFINITY;
  let safestVisibility = Number.POSITIVE_INFINITY;
  for (const party of partiesOf(world, ref)) {
    if (!actionableGroupMember(world, party)) continue;
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
 * Whether an admitted group's admission reports active.
 *
 * A party that is a user is always itself. A party that is a team reads
 * through an admission, and an admission that reports inactive reads nothing
 * here — so it is not a reader, however good its role looks.
 */
export function admissionActive(
  world: World,
  party: Party,
  ref: StoreRef,
): boolean {
  if (party.party_kind === 'user') return true;
  // The same admission `loadWorld` selects for this party: when the party is
  // scoped to a host, the admission must be from that host. Matching on team
  // id alone counted an admission from a different host as this party's,
  // and listed the group as a live reader while its name stayed unresolved.
  const entries = world.federation.filter(
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
export function readersOf(world: World, item: Item): Party[] | null {
  const store = storeOf(world, item.store);
  if (!store || store.kind !== 'team') return null;
  return partiesOf(world, store.id).filter(
    (party) =>
      admissionActive(world, party, store.id) &&
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
 * "5 people · 1 group" — a party that is a team is not a person.
 *
 * The groups half is dropped when there are none, so an all-user roster reads
 * "2 people" rather than "2 people · 0 groups".
 */
export function peopleGroups(parties: readonly Party[]): string {
  const groups = parties.filter((p) => p.party_kind !== 'user').length;
  const people = parties.length - groups;
  const head = peopleLabel(people);
  if (!groups) return head;
  return `${head} · ${groups} ${groups === 1 ? 'group' : 'groups'}`;
}
