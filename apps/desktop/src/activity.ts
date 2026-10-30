/**
 * Actionable notices listed on the Activity page alongside membership requests.
 * Each notice derives from the same snapshot state read by the screen that
 * resolves it:
 *
 * - a team with incomplete creation (`store.active === false`), resolved on the
 *   team page via Finish Setup or Remove;
 * - an inactive federated team admission (`FederationEntry.active`), resolved
 *   via Restore Access on the host team's Members tab;
 * - an unverified server or one with an expired check-in, resolved on that
 *   server's page in Settings › Account (the same servers that trigger the
 *   Settings row indicator);
 * - a team with unfinished membership work (pending membership operations and
 *   invitation recovery), resolved in the team's Unfinished activity sheet.
 *   These counts come from cached team-page reads (`useCachedUnfinishedCounts`);
 *   Activity does not issue its own reads, so unread teams are omitted.
 *
 * Background synchronization jobs are omitted here and shown in the sidebar
 * sync status control.
 */

import {
  federatedTeamName,
  plural,
  SERVER_NOT_VERIFIED,
  serverAvailability,
  serverDisplayLabel,
  serverDisplayLabelForStore,
} from './model';
import type {
  AgentSnapshot,
  FederationEntry,
  Server,
  StoreRef,
  TeamStore,
} from './model';
import type { TeamRequestCount } from './operation-queries';

export type ActivityNotice =
  | { kind: 'setup-incomplete'; id: string; store: TeamStore }
  | {
      kind: 'federation-inactive';
      id: string;
      store: TeamStore;
      entry: FederationEntry;
    }
  | {
      kind: 'server';
      id: string;
      server: Server;
      reason: 'verification-required' | 'check-in-expired';
    }
  | { kind: 'unfinished'; id: string; store: TeamStore; count: number };

/**
 * Returns activity notices in display order. `unfinished` maps each team to its
 * cached unfinished-work count; when omitted, unfinished-work notices are
 * excluded.
 */
export function activityNotices(
  snapshot: AgentSnapshot,
  unfinished: ReadonlyMap<StoreRef, number> = new Map(),
): ActivityNotice[] {
  const notices: ActivityNotice[] = [];
  const teams = snapshot.stores.filter(
    (store): store is TeamStore => store.kind === 'team',
  );
  for (const store of teams) {
    const count = unfinished.get(store.id);
    if (count)
      notices.push({
        kind: 'unfinished',
        id: `unfinished:${store.id}`,
        store,
        count,
      });
  }
  for (const store of teams) {
    if (store.active === false)
      notices.push({
        kind: 'setup-incomplete',
        id: `setup:${store.id}`,
        store,
      });
  }
  for (const entry of snapshot.federation) {
    if (entry.active) continue;
    const store = teams.find((candidate) => candidate.id === entry.store);
    if (!store) continue;
    notices.push({
      kind: 'federation-inactive',
      id: `federation:${entry.store}:${entry.remote_host_id_hex}:${entry.remote_team_id_hex}`,
      store,
      entry,
    });
  }
  for (const server of snapshot.servers) {
    const availability = serverAvailability(snapshot, server);
    if (
      !availability.available &&
      (availability.reason === 'verification-required' ||
        availability.reason === 'check-in-expired')
    )
      notices.push({
        kind: 'server',
        id: `server:${server.profileName}`,
        server,
        reason: availability.reason,
      });
  }
  return notices;
}

/** Formats the title and subtitle for an activity notice row. */
export function activityNoticeText(
  snapshot: AgentSnapshot,
  notice: ActivityNotice,
): { title: string; detail: string } {
  switch (notice.kind) {
    case 'setup-incomplete':
      return {
        title: `${notice.store.name} is inactive`,
        detail: `Team setup incomplete · ${serverDisplayLabelForStore(snapshot, notice.store)}`,
      };
    case 'unfinished':
      return {
        title: `${notice.store.name} has unfinished changes`,
        detail: `${notice.count} incomplete membership ${notice.count === 1 ? 'change' : 'changes'}`,
      };
    case 'federation-inactive':
      return {
        title: `${federatedTeamName(snapshot, notice.entry)} cannot access ${notice.store.name}`,
        detail: `Membership in ${notice.store.name} inactive`,
      };
    case 'server':
      return notice.reason === 'check-in-expired'
        ? {
            title: `${serverDisplayLabel(notice.server)} check-in expired`,
            detail: 'Stores on this server are locked',
          }
        : {
            title: `${serverDisplayLabel(notice.server)} is not verified`,
            detail: SERVER_NOT_VERIFIED.hint,
          };
  }
}

/** Sums known request counts and reports whether any count is a lower bound. */
export function requestTotal(counts: ReadonlyMap<StoreRef, TeamRequestCount>): {
  total: number;
  truncated: boolean;
} {
  let total = 0;
  let truncated = false;
  for (const count of counts.values()) {
    truncated ||= typeof count === 'string';
    total += typeof count === 'string' ? Number.parseInt(count, 10) : count;
  }
  return { total, truncated };
}

/**
 * Computes the sidebar Activity badge from known membership request counts and
 * activity notices. Teams without loaded request counts contribute 0.
 */
export function activityBadge(
  snapshot: AgentSnapshot,
  counts: ReadonlyMap<StoreRef, TeamRequestCount>,
  unfinished?: ReadonlyMap<StoreRef, number>,
): { label: string; description: string } | null {
  const { total: requests, truncated } = requestTotal(counts);
  const notices = activityNotices(snapshot, unfinished).length;
  const total = requests + notices;
  if (!total) return null;
  const parts = [
    requests ? plural(requests, 'request') : null,
    notices ? plural(notices, 'notice') : null,
  ].filter(Boolean);
  return {
    label: truncated ? `${total}+` : String(total),
    description: `${truncated ? 'At least ' : ''}${parts.join(' and ')}`,
  };
}
