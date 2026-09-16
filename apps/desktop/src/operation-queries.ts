/** Only public recovery metadata enters the query repository. Invitation payloads remain local. */
import { useCallback, useMemo } from 'react';
import { enqueueProfileWork } from './bridge';
import type { Bridge, PendingOperation } from './bridge';
import type { TeamStore } from './model';
import { useDeviceCache } from './device-cache';
import { useMetadataQuery, useQueryRepository } from './query-hooks';
import type { QueryRepository } from './query-repository';
import type { InvitationReply } from './invitation-contract';

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

export function pendingOperationsQuery(
  repository: QueryRepository,
  bridge: Bridge,
  profile: string,
) {
  return repository.query<PendingOperation[]>(
    pendingOperationKey(profile),
    () =>
      enqueueProfileWork(bridge, profile, () =>
        bridge.listPendingOperations(profile),
      ),
  );
}

const rows = (value: InvitationReply) =>
  Array.isArray(value) ? value : (value.rows ?? [value]);

export function invitationRecoveryQuery(
  repository: QueryRepository,
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

export function usePendingGroupOperations(
  bridge: Bridge,
  store: TeamStore | null,
  enabled: boolean,
) {
  const devices = useDeviceCache();
  const repository = useQueryRepository(bridge, devices?.repository);
  const query =
    store && enabled
      ? pendingOperationsQuery(repository, bridge, store.server)
      : null;
  const state = useMetadataQuery(query);
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
