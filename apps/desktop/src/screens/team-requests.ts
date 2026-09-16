/**
 * What this session has learned about pending team membership requests.
 *
 * There is no already-loaded snapshot fact for a request count: the agent
 * answers it only through `invitation`'s `inbox` action, and today that is
 * only asked for the team page currently open. Asking it for every team from
 * the rail, on every render, would be a new request list per team, repeated
 * across the whole app rather than paid for once by the page that already
 * asks it — so this registry does not ask anything itself. It only remembers
 * what `InvitationPanel` already reports through `onRowsChange` when a team's
 * own page loads its request list, the same way `railChatUnread` counts a
 * team's inbox once the chat service has already loaded it and stays silent
 * about a team whose inbox has not loaded yet.
 */

import type { Bridge } from '../bridge';
import type { AgentSnapshot, StoreRef } from '../model';
import { plural } from '../model';

export class TeamRequestRegistry {
  private counts: ReadonlyMap<StoreRef, number> = new Map();
  private listeners = new Set<() => void>();

  getSnapshot = (): ReadonlyMap<StoreRef, number> => this.counts;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  /** Records a team's own request count, as `InvitationPanel` reports it. */
  report(store: StoreRef, count: number): void {
    if (this.counts.get(store) === count) return;
    const next = new Map(this.counts);
    next.set(store, count);
    this.counts = next;
    for (const listener of this.listeners) listener();
  }
}

const registries = new WeakMap<Bridge, TeamRequestRegistry>();

/** One registry per bridge, so separate windows and tests never share state. */
export function teamRequestRegistry(bridge: Bridge): TeamRequestRegistry {
  let registry = registries.get(bridge);
  if (!registry) {
    registry = new TeamRequestRegistry();
    registries.set(bridge, registry);
  }
  return registry;
}

/**
 * The Teams tab's rail badge: the sum of every named team's request count
 * this session already knows, in `railChatUnread`'s own shape. `undefined`
 * counts — a team never visited — contribute nothing and do not suppress the
 * total the way a still-loading chat inbox does, because there is no
 * background load in flight for them to wait on.
 */
export function teamRequestsBadge(
  snapshot: AgentSnapshot,
  counts: ReadonlyMap<StoreRef, number>,
): { label: string; description: string } | null {
  let total = 0;
  for (const store of snapshot.stores) {
    if (store.kind !== 'team' || store.team_kind !== 'named') continue;
    total += counts.get(store.id) ?? 0;
  }
  if (!total) return null;
  return {
    label: String(total),
    description: `${plural(total, 'request')} to join a team`,
  };
}
