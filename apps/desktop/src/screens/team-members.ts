import { partiesOf } from '../model';
import type { AgentSnapshot, FederationEntry, Party, Store } from '../model';

/**
 * The federated teams that are members here, each one drawn once.
 *
 * A federated membership is normally an entry and the roster party it matches, listed as
 * the entry. A roster party that matches no entry is a missing record, and one
 * that matches several is an ambiguous record: both are still members of this
 * group, so they are listed as the party the roster holds, and the entries an
 * ambiguous party matches are left to that one row rather than repeated.
 */
interface FederatedTeamMembers {
  entries: FederationEntry[];
  unmatched: Party[];
  ambiguous: Party[];
}

export function federatedTeamMembers(
  snapshot: AgentSnapshot,
  store: Store,
): FederatedTeamMembers {
  const entries = snapshot.federation.filter(
    (entry) => entry.store === store.id,
  );
  const unmatched: Party[] = [];
  const ambiguous: Party[] = [];
  const claimed = new Set<FederationEntry>();
  for (const party of partiesOf(snapshot, store.id)) {
    if (party.party_kind === 'user') continue;
    const matches = entries.filter(
      (entry) =>
        entry.remote_team_id_hex === party.party_id_hex &&
        (!party.scoped_host_id_hex ||
          entry.remote_host_id_hex === party.scoped_host_id_hex),
    );
    if (!matches.length) unmatched.push(party);
    else if (matches.length > 1) {
      ambiguous.push(party);
      for (const entry of matches) claimed.add(entry);
    }
  }
  return {
    entries: entries.filter((entry) => !claimed.has(entry)),
    unmatched,
    ambiguous,
  };
}

/**
 * How many members the group has: its people and machines, plus the federated
 * groups counted once each, however their records read.
 */
export function memberCountOf(snapshot: AgentSnapshot, store: Store): number {
  const { entries, unmatched, ambiguous } = federatedTeamMembers(
    snapshot,
    store,
  );
  return (
    partiesOf(snapshot, store.id).filter((party) => party.party_kind === 'user')
      .length +
    entries.length +
    unmatched.length +
    ambiguous.length
  );
}
