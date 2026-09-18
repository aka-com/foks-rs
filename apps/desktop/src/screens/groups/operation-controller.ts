import { useCallback } from 'react';
import type { Bridge, PendingOperation } from '../../bridge';
import type { TeamStore } from '../../model';
import type { MutationFailureHandler } from '../../mutation-recovery';
import { usePendingGroupOperations } from '../../operation-queries';
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
}: {
  bridge: Bridge;
  store: TeamStore | null;
  enabled: boolean;
  onSnapshotApplied: (message: string) => Promise<void>;
  onSnapshotMutationError: MutationFailureHandler;
  onRefreshError: (error: unknown) => void;
}) {
  const { operations, refresh, generation } = usePendingGroupOperations(
    bridge,
    store,
    enabled,
  );
  const onApplied = useCallback(
    async (message: string): Promise<void> => {
      const observed = generation();
      try {
        await onSnapshotApplied(message);
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
  const mutate = async (
    action: () => Promise<unknown>,
    message: string,
  ): Promise<void> => {
    const result = await attemptMutation(
      { kind: 'resumable', operation: 'group-membership-or-admission' },
      action,
      () => onApplied(message),
    );
    await reportMutationOutcome(result, onMutationError, onRefreshError);
  };
  const resumeMembership = (operation: PendingOperation): void => {
    if (!store) return;
    const resume = membershipResume(bridge, store, operation);
    if (resume) void mutate(resume.run, resume.message);
  };
  const resumeCreation = (): void => {
    if (store)
      void mutate(
        () => bridge.resumeGroupCreation(store.id),
        'Team creation resumed',
      );
  };
  const resumeAdmission = (operationId: string): void => {
    if (store)
      void mutate(
        () => bridge.rerunGroupAdmission(store.id, operationId),
        'Team access restored',
      );
  };
  return {
    operations,
    onApplied,
    onMutationError,
    resumeMembership,
    resumeCreation,
    resumeAdmission,
  };
}
