import { refreshActivitiesFor } from '../refresh-activity';
import { useCallback, useEffect, useRef, useState } from 'react';
import {
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
import { DeviceCache } from '../device-cache';
import { DeviceMetadata } from '../device-metadata';
import { deviceAlertRegistry } from '../screens/device-alert';

/**
 * How long to wait before preloading devices from a partial catalog. The
 * window still waits for the catalog and initial device reads to settle;
 * this deadline starts useful work early rather than bypassing that gate.
 */
export const FIRST_PAINT_DEADLINE_MS = 2500;

/** What the loading screen says about the catalog read behind it. */
export interface BootProgress {
  /** Profiles that have listed both their accounts and their teams. */
  ready: number;
  /** Profiles the read is walking. */
  total: number;
  /** Initial device reads finish before the shell takes ownership. */
  devices?: { ready: number; total: number };
}

function bootProgressOf(partial: AgentSnapshot): BootProgress {
  return {
    ready: partial.catalogProfiles.filter((profile) => {
      const inventory = partial.profileInventory.find(
        (entry) => entry.profile === profile,
      );
      return (
        inventory?.accounts === 'complete' && inventory.teams === 'complete'
      );
    }).length,
    total: partial.catalogProfiles.length,
  };
}

/**
 * Whether this partial is worth the first paint: every profile has listed its
 * accounts and its teams, and no store is still loading. A read that reports
 * no profiles is vacuously ready — there is nothing left to wait for.
 */
function firstPaintReady(partial: AgentSnapshot): boolean {
  return (
    profileInventoryComplete(partial, 'accounts') &&
    profileInventoryComplete(partial, 'teams') &&
    !partial.storeInventory.some((entry) => entry.status === 'loading')
  );
}

export function useAppBootstrap(
  agentSnapshot?: AgentSnapshot,
  bridge?: Bridge,
  firstPaintDeadlineMs: number = FIRST_PAINT_DEADLINE_MS,
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
  const [bootProgress, setBootProgress] = useState<BootProgress | null>(null);
  const [deviceCache, setDeviceCache] = useState<DeviceCache | undefined>();
  const [lockState, setLockState] = useState<AppLockState | null>(null);
  const [lockError, setLockError] = useState<string | null>(null);
  const [unlocking, setUnlocking] = useState(false);
  const [bootEpoch, setBootEpoch] = useState(0);
  const bootGeneration = useRef(0);
  const publishedBootGeneration = useRef(0);
  /**
   * Tracks the active boot catalog load. Retiring a load prevents publication
   * but does not stop backend processing, which continues to hold profile
   * admission. Callers that would retire the load should wait for it to finish.
   */
  const bootRead = useRef<Promise<void> | null>(null);
  const awaitBootRead = useCallback(
    (): Promise<void> => bootRead.current ?? Promise.resolve(),
    [],
  );
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
    let painted = false;
    let latest: AgentSnapshot | null = null;
    let deadline: ReturnType<typeof setTimeout> | undefined;
    let startupCache: DeviceCache | undefined;
    let startupDevices: DeviceMetadata | undefined;
    let stopDeviceProgress: (() => void) | undefined;
    let mounting: Promise<void> | undefined;
    let deviceReadinessError: unknown;
    let catalogFailed = false;
    setBootProgress(null);
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
        const requested = initialScene(selected).location.kind === 'first-run';
        // Readiness comes first and alone: a catalog read issued before the
        // agent reports ready fails with bootstrap-required rather than
        // waiting.
        const status = await controller.establish();
        if (!alive || bootInvalidated) return;
        const current = (): boolean =>
          alive && !bootInvalidated && generation === bootGeneration.current;
        // appInfo shares no state with the catalog read, so the two run
        // together and the managed profile is published as soon as it lands.
        // The rejection handler keeps a failure that loses the race to a
        // catalog failure from surfacing as an unhandled rejection; the await
        // below still reports it.
        const info = refreshActivitiesFor(selected).run(
          'Loading application information',
          () => selected.appInfo(),
          current,
        );
        void info.then(
          (resolved) => {
            if (current()) setManagedProfile(resolved.managedProfile ?? null);
          },
          () => undefined,
        );
        const prepareFirstRun = (
          snapshot: AgentSnapshot,
          appInfo: Awaited<ReturnType<Bridge['appInfo']>>,
        ): void => {
          if (
            !maintenanceOwner.handedOff &&
            !requested &&
            profileInventoryComplete(snapshot, 'accounts') &&
            !snapshot.stores.some((entry) => entry.kind === 'account')
          ) {
            const localProfile =
              appInfo.managedProfile &&
              snapshot.servers.some(
                (server) =>
                  server.profileName === appInfo.managedProfile &&
                  serverAvailability(snapshot, server).available,
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
        };
        const publish = (snapshot: AgentSnapshot): void => {
          if (!maintenanceOwner.handoff(current)) return;
          painted = true;
          if (deadline !== undefined) {
            clearTimeout(deadline);
            deadline = undefined;
          }
          publishedBootGeneration.current = generation;
          setLoaded(snapshot);
          setLoadError(null);
        };
        const mount = (snapshot: AgentSnapshot): Promise<void> => {
          latest = snapshot;
          if (painted) {
            publish(snapshot);
            return Promise.resolve();
          }
          if (mounting) return mounting;
          startupCache ??= new DeviceCache(selected);
          startupCache.snapshot = () => latest ?? undefined;
          startupDevices ??= new DeviceMetadata(
            startupCache,
            deviceAlertRegistry(selected),
          );
          const devices = startupDevices;
          stopDeviceProgress = devices.subscribe(() => {
            if (!current() || !latest) return;
            const statuses = devices.getSnapshot();
            setBootProgress({
              ...bootProgressOf(latest),
              devices: {
                ready: statuses.reduce((sum, entry) => sum + entry.ready, 0),
                total: statuses.reduce((sum, entry) => sum + entry.total, 0),
              },
            });
          });
          mounting = (async () => {
            // A newer partial may add accounts while device reads are in
            // flight. Cover it before handing the cache and shell over.
            while (current() && latest) {
              const candidate: AgentSnapshot = latest;
              devices.update(
                candidate,
                () => current() && controller.snapshot().state === 'ready',
                (error) => {
                  deviceReadinessError = error;
                  devices.stop();
                  startupCache?.clear();
                },
              );
              await devices.settle();
              if (deviceReadinessError) throw deviceReadinessError;
              // The deadline may start preloading, but cannot bypass accounts
              // still being discovered by the initial catalog read.
              await reading;
              if (catalogFailed) return;
              const appInfo = await info;
              if (!current() || controller.snapshot().state !== 'ready') return;
              if (latest !== candidate) continue;
              stopDeviceProgress?.();
              stopDeviceProgress = undefined;
              devices.stop();
              setDeviceCache(startupCache);
              prepareFirstRun(candidate, appInfo);
              publish(candidate);
              return;
            }
          })();
          return mounting;
        };
        const requestMount = (snapshot: AgentSnapshot): void => {
          void mount(snapshot).catch((error) => {
            if (!current()) return;
            setAgentLifecycle({ state: 'failure', error });
            setLoadError(normalizeCommandError(error).message);
          });
        };
        const publishPartial = (partial: AgentSnapshot): void => {
          if (!current() || controller.snapshot().state !== 'ready') return;
          if (mounting && !painted) {
            latest = partial;
            return;
          }
          // Without an account, the completed read and appInfo must choose
          // automatic onboarding before the shell initializes its scene.
          // A deadline must not hand off that decision either.
          if (
            !painted &&
            !requested &&
            !partial.stores.some((entry) => entry.kind === 'account')
          ) {
            latest = partial;
            setBootProgress(bootProgressOf(partial));
            return;
          }
          if (painted || firstPaintReady(partial)) {
            requestMount(partial);
            return;
          }
          // Hold the loading screen and report the read's progress into it,
          // rather than mounting a shell of stores that all read "loading".
          latest = partial;
          setBootProgress(bootProgressOf(partial));
          deadline ??= setTimeout(() => {
            deadline = undefined;
            if (!latest || painted) return;
            if (!current() || controller.snapshot().state !== 'ready') return;
            if (
              !requested &&
              !latest.stores.some((entry) => entry.kind === 'account')
            )
              return;
            requestMount(latest);
          }, firstPaintDeadlineMs);
        };
        // The agent is up and the read is what the window is now waiting on, so
        // the loading screen says so before the first profile reports.
        setBootProgress({ ready: 0, total: 0 });
        let next: AgentSnapshot;
        let settleBootRead = (): void => undefined;
        const reading = new Promise<void>((resolve) => {
          settleBootRead = resolve;
        });
        bootRead.current = reading;
        try {
          next = await loadSnapshot(
            selected,
            undefined,
            undefined,
            publishPartial,
            current,
            false,
            status,
          );
        } catch (error) {
          catalogFailed = true;
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
          catalogFailed = false;
          next = emptySnapshot(status);
        } finally {
          if (bootRead.current === reading) bootRead.current = null;
          settleBootRead();
        }
        // Teams granted to an account after its first-run setup are found by
        // the scheduler's discovery job shortly after the shell mounts, not
        // here: discover_groups is a mutation, so the read that followed it
        // re-walked every store's first page past the agent's retained cache.
        if (!current()) return;
        const appInfo = await info;
        if (!current()) return;
        prepareFirstRun(next, appInfo);
        if (!alive) return;
        // Transfer maintenance ingestion to VaultShell. Its listener is
        // installed before querying the replayable native snapshot, so events
        // in this handoff window are recovered without two listeners racing
        // the same controller revision.
        await mount(next);
      } catch (error) {
        if (!alive || bootInvalidated || generation !== bootGeneration.current)
          return;
        if (
          maintenanceOwner.handedOff &&
          !isAgentReadinessError(normalizeCommandError(error))
        )
          return;
        if (!maintenanceOwner.handedOff) {
          stopDeviceProgress?.();
          stopDeviceProgress = undefined;
          startupDevices?.stop();
          startupCache?.clear();
        }
        setAgentLifecycle({ state: 'failure', error });
        setLoadError(normalizeCommandError(error).message);
      } finally {
        // The read is over: it either mounted the shell or failed into the
        // error card, so the first-paint deadline has nothing left to release.
        if (deadline !== undefined) {
          clearTimeout(deadline);
          deadline = undefined;
        }
      }
    })();
    return () => {
      alive = false;
      if (deadline !== undefined) clearTimeout(deadline);
      stopDeviceProgress?.();
      startupDevices?.stop();
      startupCache?.clear();
      stopLifecycle?.();
      maintenanceOwner.retire();
    };
  }, [
    bootEpoch,
    bridge,
    agentSnapshot,
    maintenanceOwnership,
    firstPaintDeadlineMs,
  ]);

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
    bootProgress,
    deviceCache,
    lockState,
    lockError,
    unlocking,
    agentLifecycle,
    agentController,
    maintenanceOwnership,
    lockNow,
    retireBoot,
    awaitBootRead,
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
