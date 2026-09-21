/** Only public recovery metadata enters the query repository. Invitation payloads remain local. */
import { useCallback, useEffect, useMemo } from 'react';
import { enqueueProfileWork } from './bridge';
import type { Bridge, PendingOperation } from './bridge';
import { plural } from './model';
import type { AgentSnapshot, StoreRef, TeamStore } from './model';
import { INVITATION_ACTIVITY } from './invitation-activity';
import type { InvitationActivityDetail } from './invitation-activity';
import { useDeviceCache } from './device-cache';
import {
  useMetadataQueries,
  useMetadataQuery,
  useMetadataRepository,
} from './query-hooks';
import type { MetadataRepository } from './metadata-repository';
import type { InvitationReply } from './invitation-contract';
import { manageReason } from './screens/group-model';

export const pendingOperationKey = (profile: string) =>
  ['pending-operations', profile] as const;
export const invitationRecoveryKey = (store: TeamStore) =>
  [
    'invitation-recovery',
    store.server,
    store.account,
    store.team_id_hex,
    store.alias,
  ] as const;
export const teamRequestCountKey = (store: TeamStore) =>
  [
    'team-requests',
    store.server,
    store.account,
    store.team_id_hex,
    store.alias,
  ] as const;

export function pendingOperationsQuery(
  repository: MetadataRepository,
  bridge: Bridge,
  profile: string,
) {
  return repository.query<PendingOperation[]>(
    pendingOperationKey(profile),
    async () => {
      const operations = await enqueueProfileWork(bridge, profile, () =>
        bridge.listPendingOperations(profile),
      );
      return operations.map(({ kind, alias, target }) => ({
        kind,
        alias,
        ...(target === undefined ? {} : { target }),
      }));
    },
  );
}

const rows = (value: InvitationReply) =>
  Array.isArray(value) ? value : (value.rows ?? [value]);

export function invitationRecoveryQuery(
  repository: MetadataRepository,
  bridge: Bridge,
  store: TeamStore,
) {
  // Capture public identity values, never a panel or its secret-bearing state.
  const { server, account, alias, team_id_hex } = store;
  return repository.query<number>(invitationRecoveryKey(store), async () => {
    const [operations, approvals] = await Promise.all([
      bridge.invitation(server, account, { action: 'list' }, null),
      bridge.invitation(
        server,
        account,
        { action: 'pending-approvals', team_alias: alias },
        null,
      ),
    ]);
    // Full replies can contain invitation tokens. Retain only the count.
    return (
      rows(operations).filter(
        (row) => row.team_id === team_id_hex && row.state !== 'cancelled',
      ).length +
      rows(approvals).filter(
        (row) => row.request_id && row.state !== 'complete',
      ).length
    );
  });
}

/**
 * How many membership requests wait on a team, as one shared row per team.
 * The inbox rows can carry tokens, so only their number is retained; the
 * team page's own panel keeps the rows it shows.
 */
export function teamRequestCountQuery(
  repository: MetadataRepository,
  bridge: Bridge,
  store: TeamStore,
) {
  const { server, account, alias } = store;
  return repository.query<number>(teamRequestCountKey(store), async () => {
    // `invitation` owns its own profile admission: the native bridge queues
    // the request itself. Queuing here as well would hold the profile's
    // slot while awaiting a request that cannot start until the slot is
    // released, and the inner request would expire at the admission
    // deadline instead of running.
    const reply = await bridge.invitation(
      server,
      account,
      { action: 'inbox', team_alias: alias },
      null,
    );
    return Array.isArray(reply) ? reply.length : (reply.rows?.length ?? 0);
  });
}

/** The named teams this account can manage: the only ones a request can be asked of. */
function manageableTeams(snapshot: AgentSnapshot): TeamStore[] {
  return snapshot.stores.filter(
    (store): store is TeamStore =>
      store.kind === 'team' &&
      store.team_kind === 'named' &&
      manageReason(snapshot, store, 'roster') === undefined,
  );
}

/**
 * Loads request counts for each manageable named team and shares the result
 * across subscribers. Failed team reads contribute no count and report once
 * through `onError`. Account invitation activity invalidates the team rows.
 */
export function useTeamRequestCounts(
  bridge: Bridge,
  snapshot: AgentSnapshot,
  onError?: (error: unknown) => void,
  /**
   * The repository to read through, for a caller above the shell's provider
   * such as the shell itself; below it, the provided one is used.
   */
  repository?: MetadataRepository,
): ReadonlyMap<StoreRef, number> {
  const devices = useDeviceCache();
  const shared = useMetadataRepository(
    bridge,
    repository ?? devices?.repository,
  );
  const candidates = manageableTeams(snapshot);
  // The list keeps its identity while the same teams are in it, so a
  // snapshot that changed something else does not resubscribe every row.
  const signature = candidates.map((store) => store.id).join('|');
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const teams = useMemo(() => candidates, [signature]);
  const queries = useMemo(
    () => teams.map((store) => teamRequestCountQuery(shared, bridge, store)),
    [teams, shared, bridge],
  );
  const states = useMetadataQueries(queries, onError ? { onError } : {});
  useEffect(() => {
    const refresh = (event: Event) => {
      const scope = (event as CustomEvent<InvitationActivityDetail>).detail;
      shared.invalidate(
        scope?.profile && scope.account
          ? ['team-requests', scope.profile, scope.account]
          : ['team-requests'],
      );
    };
    window.addEventListener(INVITATION_ACTIVITY, refresh);
    return () => window.removeEventListener(INVITATION_ACTIVITY, refresh);
  }, [shared]);
  return useMemo(
    () =>
      new Map(
        teams.flatMap((store, index) => {
          const count = states[index]?.data;
          return count === undefined ? [] : [[store.id, count] as const];
        }),
      ),
    [teams, states],
  );
}

/**
 * The Teams tab's rail badge: the sum of every named team's request count,
 * in `railChatUnread`'s own shape. A team whose count is not known yet
 * contributes nothing and does not suppress the total.
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

export function usePendingGroupOperations(
  bridge: Bridge,
  store: TeamStore | null,
  enabled: boolean,
  /** Reports a pending-operation query failure once to the caller. */
  onError?: (error: unknown) => void,
) {
  const devices = useDeviceCache();
  const repository = useMetadataRepository(bridge, devices?.repository);
  const query =
    store && enabled
      ? pendingOperationsQuery(repository, bridge, store.server)
      : null;
  const state = useMetadataQuery(query, onError ? { onError } : {});
  const alias = store?.alias;
  const operations = useMemo(
    () =>
      (state.data ?? []).filter(
        (row) =>
          row.alias === alias &&
          (row.kind === 'team-member-addition' ||
            row.kind === 'team-member-edit'),
      ),
    [alias, state.data],
  );
  const generation = useCallback(
    () => query?.getSnapshot().invalidation,
    [query],
  );
  const refresh = useCallback(
    async (observed?: number): Promise<void> => {
      if (!query) return;
      if (
        observed === undefined ||
        observed === query.getSnapshot().invalidation
      )
        query.invalidate();
      // A recovery-banner read never changes a previously confirmed write result.
      await query.load().catch(() => undefined);
    },
    [query],
  );
  return { operations, refresh, generation };
}
