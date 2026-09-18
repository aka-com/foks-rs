import { useCallback, useEffect, useRef, useState } from 'react';
import {
  discoverUnboundTeams,
  isAgentReadinessError,
  loadSnapshot,
  normalizeCommandError,
  selectBridge,
} from '../bridge';
import type { AppLockState, Bridge } from '../bridge';
import {
  AgentLifecycleController,
  type AgentLifecycle,
} from '../agent-lifecycle';
import { profileInventoryComplete, serverAvailability } from '../model';
import type { AgentSnapshot } from '../model';
import { initialScene } from './scenes';
import { MaintenanceOwnership } from './maintenance-ownership';

export function useAppBootstrap(
  agentSnapshot?: AgentSnapshot,
  bridge?: Bridge,
) {
  const [activeBridge, setActiveBridge] = useState<Bridge | null>(
    () => bridge ?? null,
  );
  const [loaded, setLoaded] = useState<AgentSnapshot | null>(
    () => agentSnapshot ?? null,
  );
  const [loadError, setLoadError] = useState<string | null>(null);
  const [firstRunStart, setFirstRunStart] = useState<'who' | 'local' | null>(
    null,
  );
  const [managedProfile, setManagedProfile] = useState<string | null>(null);
  const [lockState, setLockState] = useState<AppLockState | null>(null);
  const [lockError, setLockError] = useState<string | null>(null);
  const [unlocking, setUnlocking] = useState(false);
  const [bootEpoch, setBootEpoch] = useState(0);
  const bootGeneration = useRef(0);
  const publishedBootGeneration = useRef(0);
  const retireBoot = useCallback(() => {
    bootGeneration.current++;
  }, []);
  const currentBootSnapshot = useCallback(
    () =>
      Boolean(agentSnapshot) ||
      publishedBootGeneration.current === bootGeneration.current,
    [agentSnapshot],
  );
  const [agentLifecycle, setAgentLifecycle] = useState<AgentLifecycle>({
    state: 'checking',
  });
  const [agentController, setAgentController] =
    useState<AgentLifecycleController | null>(null);
  const [maintenanceOwnership] = useState(() => new MaintenanceOwnership());

  useEffect(() => {
    if (agentSnapshot) {
      setLoaded(agentSnapshot);
      if (bridge) {
        const controller = new AgentLifecycleController(
          bridge,
          agentSnapshot.agent,
          () => bridge.autoRecoverAgent?.() ?? bridge.agentStatus(),
        );
        setAgentController(controller);
        setAgentLifecycle(controller.snapshot());
        setActiveBridge(bridge);
        setLoadError(null);
        return;
      }
      setLoadError(
        'An injected snapshot must include the bridge that answers its item actions.',
      );
      return;
    }
    let alive = true;
    const generation = ++bootGeneration.current;
    const maintenanceOwner = maintenanceOwnership.acquire();
    let bootInvalidated = false;
    let restartScheduled = false;
    let stopLifecycle: (() => void) | undefined;
    const restartFromMaintenanceSnapshot = (): void => {
      bootInvalidated = true;
      if (restartScheduled) return;
      restartScheduled = true;
      setBootEpoch((value) => value + 1);
    };
    void (async () => {
      try {
        const selected = bridge ?? (await selectBridge());
        if (!alive) return;
        const controller = new AgentLifecycleController(
          selected,
          undefined,
          () => selected.autoRecoverAgent?.() ?? selected.agentStatus(),
        );
        stopLifecycle = controller.subscribe((state) => {
          if (alive) setAgentLifecycle(state);
        });
        if (alive) setAgentController(controller);
        maintenanceOwner.install(
          await selected.onMaintenanceStatus((snapshot) => {
            if (!alive || !maintenanceOwner.isCurrent()) return;
            const accepted = controller.applyMaintenance(snapshot);
            // Until VaultShell owns ingestion, any native maintenance
            // transition invalidates this boot attempt. Restart from the
            // replayable snapshot so an in-flight pre-maintenance catalog can
            // never be published and consumed events are not lost at handoff.
            if (accepted && snapshot.state !== 'idle')
              restartFromMaintenanceSnapshot();
          }),
        );
        if (!alive || bootInvalidated) return;
        const maintenance = await selected.clientStateMaintenanceStatus();
        if (!alive) return;
        controller.applyMaintenance(maintenance);
        const nextLockState = await selected.appLockState();
        if (!alive) return;
        setActiveBridge(selected);
        if (nextLockState.locked) {
          setLoaded(null);
          setLockState(nextLockState);
          setLockError(null);
          setLoadError(null);
          return;
        }
        setLockState(null);
        const maintenanceLifecycle = controller.snapshot();
        if (maintenanceLifecycle.state === 'maintenance') {
          setLoaded(null);
          setLoadError(null);
          return;
        }
        if (
          maintenanceLifecycle.state === 'restart-required' ||
          maintenanceLifecycle.state === 'recovery-required' ||
          maintenanceLifecycle.state === 'restoration-failed'
        ) {
          setLoaded(null);
          setLoadError(null);
          return;
        }
        const requested = initialScene().location.kind === 'first-run';
        const [status, appInfo] = await Promise.all([
          controller.establish(),
          selected.appInfo(),
        ]);
        if (!alive || bootInvalidated) return;
        setManagedProfile(appInfo.managedProfile ?? null);
        const current = (): boolean =>
          alive && !bootInvalidated && generation === bootGeneration.current;
        const publishPartial = (partial: AgentSnapshot): void => {
          if (
            !current() ||
            controller.snapshot().state !== 'ready' ||
            !partial.stores.length
          )
            return;
          if (!maintenanceOwner.handoff(current)) return;
          publishedBootGeneration.current = generation;
          setLoaded(partial);
          setLoadError(null);
        };
        let next: AgentSnapshot;
        try {
          next = await loadSnapshot(
            selected,
            undefined,
            undefined,
            publishPartial,
            current,
          );
        } catch (error) {
          const typed = normalizeCommandError(error);
          if (
            maintenanceOwner.handedOff ||
            !requested ||
            typed.code === 'bootstrap-required' ||
            typed.code === 'agent-lost' ||
            typed.code === 'version-mismatch' ||
            typed.fatal
          )
            throw error;
          next = emptySnapshot(status);
        }
        if (!current()) return;
        // On an ordinary launch, look for teams that were granted to an
        // account after its first-run setup. An explicit first-run location
        // keeps its own discovery step, so leave it untouched.
        if (!requested && profileInventoryComplete(next, 'accounts')) {
          try {
            if (maintenanceOwner.handedOff) publishPartial(next);
            if (await discoverUnboundTeams(selected, next, current)) {
              if (!current()) return;
              next = await loadSnapshot(
                selected,
                next,
                undefined,
                publishPartial,
                current,
              );
            }
          } catch {
            // Team discovery is best-effort and must not block launch.
          }
        }
        if (!current()) return;
        if (
          !maintenanceOwner.handedOff &&
          !requested &&
          profileInventoryComplete(next, 'accounts') &&
          !next.stores.some((entry) => entry.kind === 'account')
        ) {
          const localProfile =
            appInfo.managedProfile &&
            next.servers.some(
              (server) =>
                server.id === appInfo.managedProfile &&
                serverAvailability(next, server).available,
            )
              ? appInfo.managedProfile
              : null;
          if (alive) {
            setManagedProfile(appInfo.managedProfile ?? null);
            setFirstRunStart(localProfile ? 'local' : 'who');
          }
        } else if (alive) {
          setManagedProfile(appInfo.managedProfile ?? null);
        }
        if (!alive) return;
        // Transfer maintenance ingestion to VaultShell. Its listener is
        // installed before querying the replayable native snapshot, so events
        // in this handoff window are recovered without two listeners racing
        // the same controller revision.
        if (!maintenanceOwner.handoff(current)) return;
        publishedBootGeneration.current = generation;
        setLoaded(next);
        setLoadError(null);
      } catch (error) {
        if (!alive || bootInvalidated || generation !== bootGeneration.current)
          return;
        if (
          maintenanceOwner.handedOff &&
          !isAgentReadinessError(normalizeCommandError(error))
        )
          return;
        setAgentLifecycle({ state: 'failure', error });
        setLoadError(normalizeCommandError(error).message);
      }
    })();
    return () => {
      alive = false;
      stopLifecycle?.();
      maintenanceOwner.retire();
    };
  }, [bootEpoch, bridge, agentSnapshot, maintenanceOwnership]);

  // Re-arm the app lock from the shell. The command layer arms the lock and
  // returns its state; on a platform that cannot authenticate, `locked` stays
  // false and nothing is torn down.
  const lockNow = useCallback(async (): Promise<boolean> => {
    if (!activeBridge) return false;
    retireBoot();
    const next = await activeBridge.lockApp();
    if (!next.locked) return false;
    setLoaded(null);
    setLockState(next);
    return true;
  }, [activeBridge, retireBoot]);

  const unlock = (): void => {
    if (!activeBridge) return;
    setUnlocking(true);
    setLockError(null);
    void activeBridge
      .unlockApp()
      .then(
        (next) => {
          if (next.locked) {
            setLockState(next);
            return;
          }
          setLockState(null);
          setActiveBridge(null);
          setBootEpoch((value) => value + 1);
        },
        (error) => setLockError(normalizeCommandError(error).message),
      )
      .finally(() => setUnlocking(false));
  };
  const retry = (): void => {
    setLoadError(null);
    setLoaded(null);
    setActiveBridge(null);
    setBootEpoch((value) => value + 1);
  };
  const retryRestoration = (): void => {
    const controller = agentController;
    if (!controller) return;
    void controller
      .establish(true)
      .then(() => setBootEpoch((value) => value + 1))
      .catch((error) => {
        // A successful native restoration publishes a newer
        // maintenance snapshot while retryAgentConnection is
        // still awaited. That transition intentionally makes
        // this older establish attempt stale; the startup
        // listener above owns the boot continuation.
        const current = controller.snapshot();
        if (current.state === 'checking' || current.state === 'ready') return;
        setLoadError(normalizeCommandError(error).message);
      });
  };

  return {
    activeBridge,
    loaded,
    loadError,
    firstRunStart,
    managedProfile,
    lockState,
    lockError,
    unlocking,
    agentLifecycle,
    agentController,
    maintenanceOwnership,
    lockNow,
    retireBoot,
    currentBootSnapshot,
    unlock,
    retry,
    retryRestoration,
  };
}

function emptySnapshot(agent: AgentSnapshot['agent']): AgentSnapshot {
  return {
    agent,
    servers: [],
    accounts: [],
    stores: [],
    storeInventory: [],
    profileInventory: [],
    catalogProfiles: [],
    profileInventoryStatus: 'unavailable',
    items: [],
    parties: [],
    federation: [],
    groupDetailFailures: [],
    devices: [],
    yubiAccounts: [],
    cardsConnected: [],
    notifications: [],
    observedExpiredLeases: [],
    plaintext: {},
  };
}
