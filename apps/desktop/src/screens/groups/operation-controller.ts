import { useCallback } from 'react';
import type { Bridge, PendingOperation } from '../../bridge';
import type { TeamStore } from '../../model';
import type { MutationFailureHandler } from '../../mutation-recovery';
import { usePendingGroupOperations } from '../../operation-queries';
import { markProfileRostersStale } from '../../roster-staleness';
import {
  attemptMutation,
  reportMutationOutcome,
} from '../../commands/command-policy';

export function membershipResume(
  bridge: Bridge,
  store: TeamStore,
  operation: PendingOperation,
): { run: () => Promise<unknown>; message: string } | undefined {
  if (operation.alias !== store.alias) return;
  if (operation.kind === 'team-member-addition' && operation.target) {
    const username = operation.target;
    return {
      run: () =>
        bridge.resumeGroupMemberAddition({ storeId: store.id, username }),
      message: 'Member addition resumed',
    };
  }
  if (operation.kind === 'team-member-edit')
    return {
      run: () => bridge.resumeGroupMemberEdit(store.id),
      message: 'Member change resumed',
    };
}

export function useGroupOperationController({
  bridge,
  store,
  enabled,
  onSnapshotApplied,
  onSnapshotMutationError,
  onRefreshError,
  onReadError,
}: {
  bridge: Bridge;
  store: TeamStore | null;
  enabled: boolean;
  onSnapshotApplied: (message: string, profile?: string) => Promise<void>;
  onSnapshotMutationError: MutationFailureHandler;
  /** Reports failure to refresh the catalog after a successful mutation. */
  onRefreshError: (error: unknown) => void;
  /** Reports failure to load pending operations before any mutation occurs. */
  onReadError: (error: unknown) => void;
}) {
  const { operations, refresh, generation } = usePendingGroupOperations(
    bridge,
    store,
    enabled,
    onReadError,
  );
  const onApplied = useCallback(
    async (message: string, profile?: string): Promise<void> => {
      const observed = generation();
      try {
        await onSnapshotApplied(message, profile);
      } finally {
        await refresh(observed);
      }
    },
    [refresh, generation, onSnapshotApplied],
  );
  const onMutationError = useCallback<MutationFailureHandler>(
    async (error, options) => {
      const observed = generation();
      try {
        await onSnapshotMutationError(error, options);
      } finally {
        await refresh(observed);
      }
    },
    [refresh, generation, onSnapshotMutationError],
  );
  /**
   * Runs a resumable group write and reads back what it changed.
   *
   * `profile` names the one profile the read back needs; left out, the whole
   * catalog is read, which is what adding a remote team on
   * another server still needs. Every write here moves the team's roster, so
   * the profile holding it is marked stale before the write rather than after:
   * an ambiguous refusal can still leave the write applied, and the mark is
   * what stops the read back reusing the roster it already holds.
   */
  const mutate = async (
    action: () => Promise<unknown>,
    message: string,
    profile?: string,
  ): Promise<void> => {
    const result = await attemptMutation(
      { kind: 'resumable', operation: 'group-membership-change' },
      action,
      () => onApplied(message, profile),
    );
    await reportMutationOutcome(result, onMutationError, onRefreshError);
  };
  const resumeMembership = (operation: PendingOperation): void => {
    if (!store) return;
    const resume = membershipResume(bridge, store, operation);
    if (resume)
      void mutate(
        () => {
          markProfileRostersStale(store.server);
          return resume.run();
        },
        resume.message,
        store.server,
      );
  };
  const resumeCreation = (): void => {
    if (store)
      void mutate(
        () => {
          markProfileRostersStale(store.server);
          return bridge.resumeGroupCreation(store.id);
        },
        'Team creation resumed',
        store.server,
      );
  };
  const resumeFederatedMemberAdd = (operationId: string): void => {
    if (store)
      // A federated membership binds a remote team on another server, whose profile this
      // page does not know, so the read back stays the whole catalog.
      void mutate(() => {
        markProfileRostersStale(store.server);
        return bridge.rerunFederatedTeamMemberAdd(store.id, operationId);
      }, 'Team access restored');
  };
  return {
    operations,
    onApplied,
    onMutationError,
    resumeMembership,
    resumeCreation,
    resumeFederatedMemberAdd,
  };
}
