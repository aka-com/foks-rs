/**
 * What the Activity page lists besides membership requests: facts the
 * snapshot already holds that wait on the reader. Each entry is derived from
 * the same state the place that resolves it reads, so the two cannot
 * disagree:
 *
 * - a team whose creation never finished (`store.active === false`), resolved
 *   on the team's page by Finish Setup or Remove;
 * - a federated team admission that is inactive (`FederationEntry.active`),
 *   resolved by Restore Access on the host team's Members tab;
 * - a server that was never verified or whose check-in lapsed, resolved on
 *   that server's page in Settings › Account. These are the servers behind
 *   the Settings row's dot.
 *
 * Synchronization jobs are not listed: the sidebar's sync status owns them.
 */

import {
  federatedTeamName,
  plural,
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
    };

/** The notices, in the order the page lists them. */
export function activityNotices(snapshot: AgentSnapshot): ActivityNotice[] {
  const notices: ActivityNotice[] = [];
  const teams = snapshot.stores.filter(
    (store): store is TeamStore => store.kind === 'team',
  );
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

/** A notice's row: the title and the line under it. */
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
            detail: 'Verify this server before use',
          };
  }
}

/** The total of known request counts, and whether any of them is a floor. */
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
 * The sidebar's Activity count: the membership requests known so far plus
 * the notices above. A team whose count is not known yet adds nothing.
 */
export function activityBadge(
  snapshot: AgentSnapshot,
  counts: ReadonlyMap<StoreRef, TeamRequestCount>,
): { label: string; description: string } | null {
  const { total: requests, truncated } = requestTotal(counts);
  const notices = activityNotices(snapshot).length;
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
