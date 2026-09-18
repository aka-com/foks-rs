import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { RefObject } from 'react';
import type { Bridge } from '../bridge';
import type { AgentSnapshot } from '../model';
import {
  LeaseExpiryCoordinator,
  type LeaseExpiryClock,
} from '../scheduling/lease-expiry';
import type { useDesktopReconciliation } from '../use-desktop-reconciliation';
import type { CommandErrorHandler } from './catalog-runtime';

export function useAccessRuntime({
  agentSnapshot,
  latest,
  bridge,
  leaseClock,
  foregroundRefreshAllowed,
  reconciliationRef,
  refreshSnapshotRef,
  commandErrorRef,
}: {
  agentSnapshot: AgentSnapshot;
  latest: AgentSnapshot;
  bridge: Bridge;
  leaseClock: LeaseExpiryClock;
  foregroundRefreshAllowed: RefObject<boolean>;
  reconciliationRef: RefObject<ReturnType<
    typeof useDesktopReconciliation
  > | null>;
  refreshSnapshotRef: RefObject<(force?: boolean) => Promise<AgentSnapshot>>;
  commandErrorRef: RefObject<CommandErrorHandler>;
}) {
  const [observedExpiredLeases, setObservedExpiredLeases] = useState(
    agentSnapshot.observedExpiredLeases,
  );
  const [accessGenerations, setAccessGenerations] = useState<
    ReadonlyMap<string, number>
  >(() => new Map());
  const accessSession = useRef<object>({}).current;
  const expiryCoordinator = useRef<LeaseExpiryCoordinator | null>(null);
  useEffect(() => {
    const coordinator = new LeaseExpiryCoordinator(
      leaseClock,
      ({ observed, newlyExpired }) => {
        setObservedExpiredLeases([...observed]);
        if (!newlyExpired.length) return;
        setAccessGenerations((current) => {
          const next = new Map(current);
          for (const entry of newlyExpired)
            next.set(entry.profile, (next.get(entry.profile) ?? 0) + 1);
          return next;
        });
        if (foregroundRefreshAllowed.current)
          for (const entry of newlyExpired)
            reconciliationRef.current?.reconnect(entry.profile, 'recovery');
        if (foregroundRefreshAllowed.current)
          void refreshSnapshotRef
            .current(true)
            .catch((error: unknown) => commandErrorRef.current(error));
      },
    );
    expiryCoordinator.current = coordinator;
    const reconcileForeground = (): void => {
      coordinator.foreground();
      if (!bridge.native && foregroundRefreshAllowed.current)
        void refreshSnapshotRef
          .current()
          .catch((error: unknown) => commandErrorRef.current(error));
    };
    const reconcileVisible = (): void => {
      if (!document.hidden) reconcileForeground();
    };
    window.addEventListener('focus', reconcileForeground);
    window.addEventListener('pageshow', reconcileForeground);
    document.addEventListener('visibilitychange', reconcileVisible);
    return () => {
      expiryCoordinator.current = null;
      coordinator.dispose();
      window.removeEventListener('focus', reconcileForeground);
      window.removeEventListener('pageshow', reconcileForeground);
      document.removeEventListener('visibilitychange', reconcileVisible);
    };
  }, [
    leaseClock,
    bridge.native,
    foregroundRefreshAllowed,
    reconciliationRef,
    refreshSnapshotRef,
    commandErrorRef,
  ]);

  useEffect(() => {
    expiryCoordinator.current?.update(
      latest.servers,
      latest.observedExpiredLeases,
    );
  }, [latest.observedExpiredLeases, latest.servers]);

  const accessNow = useCallback(() => leaseClock.now(), [leaseClock]);
  const chatClock = useMemo(
    () => ({
      now: () => leaseClock.now() * 1_000,
      later: (callback: () => void, delayMs: number) =>
        leaseClock.later(callback, delayMs),
      cancel: (timer: unknown) => leaseClock.cancel(timer),
      random: Math.random,
    }),
    [leaseClock],
  );

  return {
    observedExpiredLeases,
    accessGenerations,
    accessSession,
    accessNow,
    chatClock,
  };
}
