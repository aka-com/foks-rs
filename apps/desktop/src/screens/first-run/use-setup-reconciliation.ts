import { useCallback, useEffect, useRef, useState } from 'react';
import { enqueueProfileWork } from '../../bridge';
import type { Bridge, PendingOperation } from '../../bridge';
import {
  classifyFirstRunFailure,
  presentFirstRunFailure,
  reconcileFirstRunFailure,
  type FirstRunFailure,
  type FirstRunOperation,
} from '../../first-run-failure';
import type { AgentSnapshot } from '../../model';
import type { useFirstRunController } from '../../use-first-run-controller';

/** Owns pending-operation inventory and reconciliation after ambiguous writes. */
export function useSetupReconciliation({
  bridge,
  agentReady,
  checkpoint,
  mounted,
  onRefreshSnapshot,
  setMessage,
  setUsername,
}: Pick<ReturnType<typeof useFirstRunController>, 'checkpoint' | 'mounted'> & {
  bridge: Bridge;
  agentReady: boolean;
  onRefreshSnapshot: (force?: boolean) => Promise<AgentSnapshot>;
  setMessage: (message: string | null) => void;
  setUsername: React.Dispatch<React.SetStateAction<string>>;
}) {
  const profile = checkpoint.profile;
  const [pending, setPending] = useState<PendingOperation[]>([]);
  const pendingRef = useRef<PendingOperation[]>([]);
  const [lastFailure, setLastFailure] = useState<FirstRunFailure | null>(null);
  const fail = useCallback(
    (
      operation: FirstRunOperation,
      error: unknown,
      report: (message: string) => void = setMessage,
      isCurrent: () => boolean = () => mounted.current,
      onFailure?: (failure: FirstRunFailure) => void,
    ): void => {
      if (!isCurrent()) return;
      const failure = classifyFirstRunFailure(operation, error);
      setLastFailure(failure);
      onFailure?.(failure);
      // The command layer sets its write gate when a first-run mutation
      // returns an ambiguous or response-binding result, and only a fresh
      // catalog load releases it. Without this refresh the user is stuck on
      // "Refresh the vault..." until the app restarts, so reconcile here and
      // re-read pending operations so a committed-but-unacknowledged signup
      // can still be resumed.
      if (failure.recovery !== 'pending') {
        report(failure.error.message);
        return;
      }
      report(presentFirstRunFailure(failure).detail);
      // A first server check can set the gate before a profile has been
      // selected. Only the pending-operation read needs a profile.
      void reconcileFirstRunFailure(failure, async () => {
        if (!isCurrent()) return;
        await onRefreshSnapshot();
        if (!isCurrent()) return;
        if (profile) {
          const rows = await enqueueProfileWork<PendingOperation[] | null>(
            bridge,
            profile.profile,
            () =>
              isCurrent()
                ? bridge.listPendingOperations(profile.profile)
                : Promise.resolve(null),
          );
          if (!isCurrent() || !rows) return;
          pendingRef.current = rows;
          setPending(rows);
        }
      }).then((reconciled) => {
        if (!isCurrent()) return;
        onFailure?.(reconciled);
        report(presentFirstRunFailure(reconciled).detail);
      });
    },
    [bridge, mounted, onRefreshSnapshot, profile, setMessage],
  );
  useEffect(() => {
    if (!agentReady || !profile) {
      if (pendingRef.current.length > 0) {
        pendingRef.current = [];
        setPending([]);
      }
      return;
    }
    let alive = true;
    void enqueueProfileWork(bridge, profile.profile, () =>
      bridge.listPendingOperations(profile.profile),
    ).then(
      (rows) => {
        if (!alive) return;
        const current = pendingRef.current;
        const unchanged =
          current.length === rows.length &&
          current.every(
            (entry, index) =>
              entry.kind === rows[index]?.kind &&
              entry.alias === rows[index]?.alias &&
              entry.target === rows[index]?.target,
          );
        if (!unchanged) {
          pendingRef.current = rows;
          setPending(rows);
        }
        if (checkpoint.returning) {
          const recoveries = rows.filter(
            (row) => row.kind === 'account-recovery' && !row.target,
          );
          if (recoveries.length === 1) {
            const resumableAlias = recoveries[0]?.alias;
            if (resumableAlias) {
              setUsername((current) => current || resumableAlias);
            }
          }
        }
      },
      (error) => {
        if (alive) fail('pending-read', error);
      },
    );
    return () => {
      alive = false;
    };
  }, [agentReady, bridge, checkpoint.returning, fail, profile, setUsername]);

  return { pending, lastFailure, fail };
}
