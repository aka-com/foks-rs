/** Only public recovery metadata enters the query repository. Invitation payloads remain local. */
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useSyncExternalStore,
} from 'react';
import {
  enqueueProfileWork,
  isAgentSessionError,
  isTerminalCommandError,
  normalizeCommandError,
} from './bridge';
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
import type { MetadataQuery, MetadataRepository } from './metadata-repository';
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
    // The agent journals these when this app starts one, and every
    // membership write invalidates the row whether it was applied or
    // refused, so it need not be re-read on the metadata cadence.
    PENDING_OPERATION_FRESHNESS,
  );
}

/** How long a profile's journal of unfinished operations is current. */
export const PENDING_OPERATION_FRESHNESS = 5 * 60_000;

const rows = (value: InvitationReply) =>
  Array.isArray(value) ? value : (value.rows ?? [value]);

/**
 * How much invitation work is unfinished for a team. Kept on the default
 * freshness window rather than the request count's: half of it counts the
 * team's own invited members, which advance with the team chain on an
 * ordinary catalog read, so this count moves without a local mutation to
 * invalidate it.
 */
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
        (row) =>
          row.team_id === team_id_hex &&
          row.state !== 'cancelled' &&
          row.state !== 'complete',
      ).length +
      rows(approvals).filter(
        (row) => row.request_id && row.state !== 'complete',
      ).length
    );
  });
}

/** A plus suffix means the server may have more pending requests. */
export type TeamRequestCount = number | `${number}+`;

function requestCount(count: number, truncated?: boolean): TeamRequestCount {
  return truncated ? `${count}+` : count;
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
  return repository.query<TeamRequestCount>(
    teamRequestCountKey(store),
    async () => {
      // `invitation` owns its own profile admission: the native bridge queues
      // the request itself. Queuing here as well would hold the profile's
      // slot while awaiting a request that cannot start until the slot is
      // released, and the inner request would expire at the admission
      // deadline instead of running.
      const fullInbox = async (): Promise<TeamRequestCount> => {
        const reply = await bridge.invitation(
          server,
          account,
          { action: 'inbox', team_alias: alias },
          null,
        );
        return Array.isArray(reply)
          ? reply.length
          : requestCount(reply.rows?.length ?? 0, reply.possibly_truncated);
      };
      let reply;
      try {
        reply = await bridge.invitation(
          server,
          account,
          { action: 'inbox-count', team_alias: alias },
          null,
        );
      } catch (error) {
        // An agent that predates the count-only action cannot decode it and
        // refuses the request. `invalid-request` also stands for argument
        // validation and an interrupted worker, and the refusal carries no
        // stable marker saying which it was, so this falls back for this
        // read only. A newer agent costs nothing; an older one costs one
        // refused request per badge read rather than a badge that silently
        // reads whole inboxes for the rest of the session.
        if (normalizeCommandError(error).code !== 'invalid-request')
          throw error;
        return fullInbox();
      }
      const count = Array.isArray(reply) ? undefined : reply.count;
      if (count === undefined) {
        throw new Error('The agent answered a request count without a count.');
      }
      return requestCount(
        count,
        !Array.isArray(reply) && reply.possibly_truncated,
      );
    },
    // A badge, read for every manageable team at unlock: invitation activity
    // invalidates it, so it need not be re-read on the metadata cadence.
    TEAM_REQUEST_FRESHNESS,
  );
}

/** How long a request count is current without invitation activity. */
export const TEAM_REQUEST_FRESHNESS = 5 * 60_000;

/**
 * Whether a failed request count is worth telling the user about. The count
 * is a badge: a server that is offline, busy or slow leaves it blank, and
 * saying so once per team per retry would be noise. What still reaches the
 * handler is what changes the session itself: the agent lost or in a state
 * that quarantines it.
 */
export function reportableTeamRequestError(error: unknown): boolean {
  const typed = normalizeCommandError(error);
  return isAgentSessionError(typed) || isTerminalCommandError(typed);
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
): ReadonlyMap<StoreRef, TeamRequestCount> {
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
  const reportError = useCallback(
    (error: unknown) => {
      if (onError && reportableTeamRequestError(error)) onError(error);
    },
    [onError],
  );
  const states = useMetadataQueries(queries, { onError: reportError });
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
  counts: ReadonlyMap<StoreRef, TeamRequestCount>,
): { label: string; description: string } | null {
  let total = 0;
  let truncated = false;
  for (const store of snapshot.stores) {
    if (store.kind !== 'team' || store.team_kind !== 'named') continue;
    const count = counts.get(store.id) ?? 0;
    truncated ||= typeof count === 'string';
    total += typeof count === 'string' ? Number.parseInt(count, 10) : count;
  }
  if (!total) return null;
  return {
    label: String(requestCount(total, truncated)),
    description: `${truncated ? 'At least ' : ''}${plural(total, 'request')} to join a team`,
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
        (row) => row.alias === alias && blocksMembership(row),
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

/** The pending-operation kinds a team's Unfinished activity band counts. */
function blocksMembership(operation: PendingOperation): boolean {
  return (
    operation.kind === 'team-member-addition' ||
    operation.kind === 'team-member-edit'
  );
}

const NO_COUNTS: ReadonlyMap<StoreRef, number> = new Map();

/**
 * Unfinished membership work per team, read only from what earlier reads
 * left in the shared repository: the profile's pending-operation journal
 * (the team's own membership operations, as its page filters them) plus the
 * team's invitation recovery count. These are the two numbers the team
 * page's Unfinished activity band adds up. A team neither has been read for
 * is left out, and a team whose total is zero is not listed.
 *
 * Nothing here reads: the queries are found with `peek` and observed with
 * `watch`, so they are neither created nor made due for reconciliation.
 * When the team page or its sheet re-reads them, the counts follow.
 */
export function useCachedUnfinishedCounts(
  bridge: Bridge,
  teams: readonly TeamStore[],
  /** As for `useTeamRequestCounts`: for a caller above the shell's provider. */
  fallback?: MetadataRepository,
): ReadonlyMap<StoreRef, number> {
  const devices = useDeviceCache();
  const repository = useMetadataRepository(
    bridge,
    fallback ?? devices?.repository,
  );
  const teamsKey = teams
    .map((team) => `${team.id}|${team.server}|${team.alias}`)
    .join(',');
  // The team list is identified by its key, so a new array with the same
  // teams keeps the subscription.
  const teamsRef = useRef(teams);
  teamsRef.current = teams;
  const cached = useRef<{
    signature: string;
    counts: ReadonlyMap<StoreRef, number>;
  }>({
    signature: '',
    counts: NO_COUNTS,
  });
  const sources = useCallback(() => {
    const operations = new Map<string, MetadataQuery<PendingOperation[]>>();
    const recovery = new Map<StoreRef, MetadataQuery<number>>();
    for (const team of teamsRef.current) {
      const journal = repository.peek<PendingOperation[]>(
        pendingOperationKey(team.server),
      );
      if (journal) operations.set(team.server, journal);
      const invitations = repository.peek<number>(invitationRecoveryKey(team));
      if (invitations) recovery.set(team.id, invitations);
    }
    return { operations, recovery };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [repository, teamsKey]);
  const subscribe = useCallback(
    (listener: () => void) => {
      let released: (() => void)[] = [];
      const attach = (): void => {
        for (const release of released) release();
        const { operations, recovery } = sources();
        released = [
          ...[...operations.values()].map((query) => query.watch(listener)),
          ...[...recovery.values()].map((query) => query.watch(listener)),
        ];
      };
      attach();
      const stopEntries = repository.watchEntries(() => {
        attach();
        listener();
      });
      return () => {
        stopEntries();
        for (const release of released) release();
      };
    },
    [repository, sources],
  );
  const getSnapshot = useCallback((): ReadonlyMap<StoreRef, number> => {
    const { operations, recovery } = sources();
    const entries: [StoreRef, number][] = [];
    for (const team of teamsRef.current) {
      const journal = operations.get(team.server)?.getSnapshot().data;
      const invitations = recovery.get(team.id)?.getSnapshot().data;
      const total =
        (journal?.filter(
          (operation) =>
            operation.alias === team.alias && blocksMembership(operation),
        ).length ?? 0) + (invitations ?? 0);
      if (total > 0) entries.push([team.id, total]);
    }
    const signature = JSON.stringify(entries);
    if (signature !== cached.current.signature)
      cached.current = {
        signature,
        counts: entries.length ? new Map(entries) : NO_COUNTS,
      };
    return cached.current.counts;
  }, [sources]);
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}
