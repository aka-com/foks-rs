import { useCallback, useEffect, useRef, useState } from 'react';
import type { Dispatch, SetStateAction } from 'react';
import type { ToastController } from '/kit/toasts';
import { AgentRecoveryController } from '../agent-recovery';
import {
  maintenanceOutcomeMessage,
  type AgentLifecycle,
  type AgentLifecycleController,
} from '../agent-lifecycle';
import {
  isAgentSessionError,
  commandRecovery,
  normalizeCommandError,
  onAgentReadinessRequired,
} from '../bridge';
import type { Bridge, CommandError } from '../bridge';
import { rememberChatLocation, type LocationStore } from '../location';
import type { Item } from '../model';
import {
  workflowForError,
  type WriteWorkflow,
} from '../screens/write-workflows';
import type { LeaseExpiryClock } from '../scheduling/lease-expiry';
import { useDesktopReconciliation } from '../use-desktop-reconciliation';
import type { CatalogRuntime } from './catalog-runtime';
import type { MaintenanceOwnership } from './maintenance-ownership';

export function useShellRuntime({
  lifetime,
  bridge,
  agentController,
  maintenanceOwnership,
  catalog,
  retireBoot,
  locations,
  locationKind,
  toasts,
  setWorkflow,
  leaseClock,
}: {
  lifetime: import('./access-lifetime').AccessLifetime;
  bridge: Bridge;
  agentController: AgentLifecycleController;
  maintenanceOwnership: MaintenanceOwnership;
  catalog: CatalogRuntime;
  retireBoot: () => void;
  locations: LocationStore;
  locationKind: ReturnType<LocationStore['getSnapshot']>['location']['kind'];
  toasts: ToastController;
  setWorkflow: Dispatch<SetStateAction<WriteWorkflow>>;
  leaseClock: LeaseExpiryClock;
}) {
  const {
    latest,
    latestRef,
    catalogGate,
    publishSnapshot,
    refreshSnapshot,
    commandErrorRef,
    setAgentCatalogReady,
    metadataReconciliation,
  } = catalog;
  const [agentLifecycle, setAgentLifecycle] = useState<AgentLifecycle>(() =>
    agentController.snapshot(),
  );
  const [concealSignal, setConcealSignal] = useState(0);
  // A conceal ends the session the remembered chat belonged to: the account
  // that comes back may not have that team on this Mac, so the rail's Chat tab
  // runs the first-team fallback again instead of reopening it. The Chat tab
  // is keyed by the same signal, so its own memory is written after this.
  useEffect(() => {
    if (!concealSignal) return;
    rememberChatLocation(null);
    locations.clearTabMemory();
  }, [concealSignal, locations]);

  const refreshSnapshotRef = useRef(refreshSnapshot);
  const foregroundRefreshAllowed = useRef(false);
  const recoveryRef = useRef<AgentRecoveryController | null>(null);
  const reconciliationRef = useRef<ReturnType<
    typeof useDesktopReconciliation
  > | null>(null);
  const healthProbe = useRef<Promise<void> | null>(null);
  refreshSnapshotRef.current = refreshSnapshot;
  foregroundRefreshAllowed.current =
    latest.agent.state === 'ready' &&
    agentController.snapshot().state === 'ready';

  const disconnectAgent = useCallback(
    (message: string): void => {
      if (!agentController.disconnect(message)) return;
      foregroundRefreshAllowed.current = false;
      retireBoot();
      lifetime.retire('disconnect');
      setAgentCatalogReady(false);
      setConcealSignal((value) => value + 1);
      if (bridge.autoRecoverAgent)
        void recoveryRef.current
          ?.recover({
            code: 'agent-lost',
            message,
            retryable: true,
            fatal: true,
            ambiguous: false,
          })
          .catch((error: unknown) => commandErrorRef.current(error));
    },
    [
      agentController,
      bridge,
      lifetime,
      retireBoot,
      setAgentCatalogReady,
      commandErrorRef,
    ],
  );

  const recoverAgentReadiness = useCallback(
    async (reconnect: boolean): Promise<void> => {
      if (
        reconnect &&
        bridge.autoRecoverAgent &&
        recoveryRef.current &&
        ['ready', 'disconnected', 'failure'].includes(
          agentController.snapshot().state,
        )
      ) {
        await recoveryRef.current.retry();
        return;
      }
      try {
        await agentController.establish(reconnect);
      } catch (error) {
        // Native restoration success publishes its maintenance transition
        // before retryAgentConnection resolves. That newer transition makes
        // this establish attempt stale; its event handler owns the follow-up
        // establish and catalog refresh. Retry failures remain disconnected.
        const state = agentController.snapshot().state;
        if (reconnect && state !== 'failure' && state !== 'disconnected')
          return;
        throw error;
      }
      // Readiness and catalog availability are separate. A catalog failure
      // leaves the connected agent ready and is reported by the caller.
      try {
        await refreshSnapshot(true);
      } catch (error) {
        if (!reconnect) throw error;
        commandErrorRef.current(error);
      }
    },
    [agentController, bridge, refreshSnapshot, commandErrorRef],
  );

  const checkAgentHealth = useCallback((): void => {
    if (!bridge.autoRecoverAgent || healthProbe.current) return;
    const recovery = recoveryRef.current;
    const generation = recovery?.captureGeneration();
    const pending = (bridge.probeAgentStatus?.() ?? bridge.agentStatus())
      .then((status) => {
        if (
          recoveryRef.current !== recovery ||
          recovery?.captureGeneration() !== generation
        )
          return;
        if (status.state === 'bootstrap')
          agentController.requireBootstrap(status.step);
      })
      .catch((error: unknown) => {
        if (
          recoveryRef.current !== recovery ||
          recovery?.captureGeneration() !== generation
        )
          return;
        const typed = normalizeCommandError(error);
        if (typed.code === 'bootstrap-required')
          agentController.requireBootstrap(
            typed.details?.reason ?? 'initialize-state',
          );
        else if (typed.code === 'agent-lost' || typed.fatal)
          commandErrorRef.current(error);
      })
      .finally(() => {
        if (healthProbe.current === pending) healthProbe.current = null;
      });
    healthProbe.current = pending;
  }, [agentController, bridge, commandErrorRef]);

  const handledSessionErrors = useRef(new WeakSet<CommandError>());
  const commandError = useCallback(
    (error: unknown, item?: Item, draft = ''): void => {
      const typed = normalizeCommandError(error);
      if (
        typed.code === 'catalog-read-retired' ||
        typed.code === 'agent-request-retired'
      )
        return;
      if (typed.code === 'deadline-exceeded') checkAgentHealth();
      const recovery = commandRecovery(typed);
      if (recovery.kind === 'quarantine' && recovery.scope === 'agent') {
        if (handledSessionErrors.current.has(typed)) return;
        handledSessionErrors.current.add(typed);
        recoveryRef.current?.cancel();
        agentController.fail(typed);
        lifetime.retire('access-change');
        setAgentCatalogReady(false);
        setConcealSignal((value) => value + 1);
        return;
      }
      if (typed.code === 'agent-lost') {
        disconnectAgent(typed.message);
        return;
      }
      const routed = workflowForError(error, item, draft);
      if (routed) {
        setWorkflow(routed);
        return;
      }
      // Catalog synchronization requests use standard toast notifications rather than warning alerts.
      toasts.show(
        typed.message,
        typed.code === 'catalog-required' ? undefined : { tone: 'warning' },
      );
    },
    [
      agentController,
      lifetime,
      checkAgentHealth,
      disconnectAgent,
      toasts,
      setWorkflow,
      setAgentCatalogReady,
    ],
  );
  commandErrorRef.current = commandError;

  useEffect(() => {
    let alive = true;
    const recovery = new AgentRecoveryController({
      lifecycle: agentController,
      isRecoveryPermitted: () => alive,
      reconcile: async (_status, isCurrent) => {
        if (isCurrent()) await refreshSnapshotRef.current(true);
        if (isCurrent()) reconciliationRef.current?.wake('recovery');
      },
    });
    recoveryRef.current = recovery;
    return () => {
      alive = false;
      recovery.dispose();
      if (recoveryRef.current === recovery) recoveryRef.current = null;
    };
  }, [agentController]);

  const reconciliation = useDesktopReconciliation({
    bridge,
    snapshot: latest,
    enabled: agentLifecycle.state === 'ready' && locationKind !== 'first-run',
    gate: catalogGate,
    retireBoot,
    current: () => latestRef.current,
    publish: publishSnapshot,
    refresh: () => refreshSnapshotRef.current(),
    metadata: () => metadataReconciliation.current(),
    report: (error) => {
      if (normalizeCommandError(error).code === 'deadline-exceeded')
        checkAgentHealth();
      else commandErrorRef.current(error);
    },
    nowSeconds: () => leaseClock.now(),
  });
  reconciliationRef.current = reconciliation;
  // Configure post-mutation refreshes to reload only the affected profile
  // through the reconciliation service.
  catalog.profileRefresh.current = (profile) =>
    reconciliation.refreshProfile(profile);
  useEffect(
    () =>
      lifetime.subscribe((event) => {
        if (event.profile === undefined)
          reconciliation.scheduler.setEnabled(false);
      }),
    [lifetime, reconciliation],
  );

  useEffect(() => {
    return agentController.subscribe(setAgentLifecycle);
  }, [agentController]);

  useEffect(() => {
    if (!bridge.native) return;
    const owner = maintenanceOwnership.acquire();
    const apply = (
      snapshot: Awaited<ReturnType<Bridge['clientStateMaintenanceStatus']>>,
    ): void => {
      if (!owner.isCurrent()) return;
      if (!agentController.applyMaintenance(snapshot)) return;
      if (snapshot.state === 'idle') return;
      foregroundRefreshAllowed.current = false;
      retireBoot();
      lifetime.retire('maintenance');
      setAgentCatalogReady(false);
      setConcealSignal((value) => value + 1);
      if (snapshot.state !== 'complete') return;
      if (snapshot.operation.status === 'failed')
        commandError(snapshot.operation.error);
      else if (snapshot.operation.status === 'completed')
        toasts.show(
          maintenanceOutcomeMessage(snapshot.operation, snapshot.kind),
        );
      if (snapshot.disposition.status === 'restoration-failed')
        commandError(snapshot.disposition.error);
      if (snapshot.disposition.status === 'continue-current-root')
        void agentController
          .establish(false)
          .then(() => refreshSnapshot(true))
          .catch(commandError);
    };
    void bridge
      .onMaintenanceStatus(apply)
      .then(async (unlisten) => {
        owner.install(unlisten);
        if (!owner.isCurrent()) return;
        apply(await bridge.clientStateMaintenanceStatus());
      })
      .catch(commandError);
    return () => owner.retire();
  }, [
    agentController,
    bridge,
    commandError,
    refreshSnapshot,
    retireBoot,
    toasts,
    maintenanceOwnership,
    lifetime,
    setAgentCatalogReady,
  ]);

  const handledReadinessErrors = useRef(new WeakSet<CommandError>());
  const handleAgentReadinessFailure = useCallback(
    (error: CommandError) => {
      if (
        !isAgentSessionError(error) ||
        handledReadinessErrors.current.has(error)
      )
        return;
      // checked() reports before rejecting with this same normalized object.
      // A component forwarding that rejection must not invalidate recovery twice.
      handledReadinessErrors.current.add(error);
      if (error.code === 'agent-lost') {
        disconnectAgent(error.message);
        return;
      }
      if (commandRecovery(error).kind === 'quarantine') {
        commandError(error);
        return;
      }
      foregroundRefreshAllowed.current = false;
      retireBoot();
      lifetime.retire('access-change');
      setAgentCatalogReady(false);
      const step = error.details?.reason ?? 'initialize-state';
      agentController.requireBootstrap(step);
      // Onboarding owns an explicit Retry setup action. Keep automatic
      // recovery for commands issued elsewhere in the shell.
      if (locationKind !== 'first-run')
        void recoverAgentReadiness(false).catch(commandError);
    },
    [
      agentController,
      commandError,
      disconnectAgent,
      recoverAgentReadiness,
      retireBoot,
      lifetime,
      locationKind,
      setAgentCatalogReady,
    ],
  );

  useEffect(
    () => onAgentReadinessRequired(handleAgentReadinessFailure),
    [handleAgentReadinessFailure],
  );

  useEffect(() => {
    let alive = true;
    let pending = false;
    const check = async (): Promise<void> => {
      if (pending) return;
      pending = true;
      try {
        const generation = recoveryRef.current?.captureGeneration();
        const message = await bridge.takeAgentConnectionLoss();
        if (
          alive &&
          message &&
          recoveryRef.current?.captureGeneration() === generation
        )
          disconnectAgent(message);
      } catch (error) {
        if (alive && normalizeCommandError(error).code === 'agent-lost')
          commandError(error);
      } finally {
        pending = false;
      }
    };
    void check();
    const timer = window.setInterval(() => void check(), 1000);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [bridge, commandError, disconnectAgent]);

  return {
    agentLifecycle,
    concealSignal,
    setConcealSignal,
    commandError,
    recoverAgentReadiness,
    handleAgentReadinessFailure,
    reconciliation,
    reconciliationRef,
    foregroundRefreshAllowed,
    refreshSnapshotRef,
  };
}

export type ShellRuntime = ReturnType<typeof useShellRuntime>;
