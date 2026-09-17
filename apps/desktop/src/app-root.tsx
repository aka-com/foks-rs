import { ChatInboxProvider } from './chat/inbox-provider';
/**
 * Root application component for the FOKS desktop vault shell.
 *
 * Integrates sidebar navigation, item screens, details panels, and modal
 * workflows with bridge IPC and URL location state.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { Dialog, OverlayProvider } from '/kit/overlay-primitives';
import { ToastController, ToastProvider } from '/kit/toasts';
import {
  discoverUnboundTeams,
  isAgentReadinessError,
  loadWorld,
  normalizeCommandError,
  onAgentReadinessRequired,
  selectBridge,
} from './bridge';
import type { AppLockState, Bridge, CommandError } from './bridge';
import {
  agentLifecycleLabel,
  AgentLifecycleController,
  maintenanceOutcomeMessage,
  type AgentLifecycle,
} from './agent-lifecycle';
import { Button } from './components';
import {
  FIRST_RUN_CHECKPOINT_KEY,
  completedFirstRunSteps,
  decodeFirstRunCheckpoint,
  firstRunStepCount,
} from './first-run-state';
import type { FirstRunCheckpoint } from './first-run-state';
import {
  LocationStore,
  decodeScene,
  sceneHref,
  sceneOf,
  storeAtScene,
  useLocationState,
} from './location';
import { INITIAL_SCENE } from './location';
import type { Scene } from './location';
import {
  applyLease,
  isLogin,
  kindOf,
  nameOf,
  notesNow,
  profileInventoryComplete,
  serverAvailability,
  storeOf,
  storeReadable,
} from './model';
import type { Item, World } from './model';
import { reconcileMutationFailure } from './mutation-recovery';
import type { MutationFailureHandler } from './mutation-recovery';
import { Sidebar } from './shell/sidebar';
import { AlertsScreen } from './screens/alerts-screen';
import { DetailsPanel } from './screens/details-panel';
import { ItemsScreen } from './screens/items-screen';
import { ChatScreen } from './screens/chat-screen';
import { GroupSettingsScreen } from './screens/groups-screen';
import {
  FirstRunChecklistStatus,
  FirstRunExperience,
} from './screens/first-run-screen';
import { PlaceholderScreen } from './screens/placeholder-screen';
import { SettingsScreen } from './screens/settings-screen';
import { listsItems } from './screens/scope';
import {
  initialWriteWorkflow,
  workflowForError,
  WriteOverlay,
} from './screens/write-workflows';
import type { WriteWorkflow } from './screens/write-workflows';
import {
  LeaseExpiryCoordinator,
  systemLeaseExpiryClock,
} from './scheduling/lease-expiry';
import type { LeaseExpiryClock } from './scheduling/lease-expiry';

/** The app's name, as the title bar and the first-run sidebar write it. */
export const APP_NAME = 'FOKS';

/** The scene the address bar asks for, or the shell's own starting point. */
function initialScene(): Scene {
  if (typeof window === 'undefined') return INITIAL_SCENE;
  return decodeScene(window.location.search);
}

function incompleteFirstRunCheckpoint(): FirstRunCheckpoint | null {
  if (typeof window === 'undefined') return null;
  try {
    const checkpoint = decodeFirstRunCheckpoint(
      window.localStorage.getItem(FIRST_RUN_CHECKPOINT_KEY),
    );
    return checkpoint &&
      completedFirstRunSteps(checkpoint) < firstRunStepCount(checkpoint)
      ? checkpoint
      : null;
  } catch {
    return null;
  }
}

export interface AppProps {
  /** Supplying a world makes render tests synchronous. Production omits it. */
  world?: World;
  /** A command seam for tests; production selects the Tauri or mock bridge. */
  bridge?: Bridge;
  /** Injected by the render tests so navigation is observable. */
  store?: LocationStore;
  /** Controlled wall clock for expiry lifecycle tests. */
  leaseClock?: LeaseExpiryClock;
}

export function App({ world, bridge, store, leaseClock }: AppProps): ReactNode {
  const [activeBridge, setActiveBridge] = useState<Bridge | null>(
    () => bridge ?? null,
  );
  const [loaded, setLoaded] = useState<World | null>(() => world ?? null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [firstRunStart, setFirstRunStart] = useState<'who' | 'local' | null>(
    null,
  );
  const [managedProfile, setManagedProfile] = useState<string | null>(null);
  const [lockState, setLockState] = useState<AppLockState | null>(null);
  const [lockError, setLockError] = useState<string | null>(null);
  const [unlocking, setUnlocking] = useState(false);
  const [bootEpoch, setBootEpoch] = useState(0);
  const [agentLifecycle, setAgentLifecycle] = useState<AgentLifecycle>({
    state: 'checking',
  });
  const [agentController, setAgentController] =
    useState<AgentLifecycleController | null>(null);

  useEffect(() => {
    if (world) {
      setLoaded(world);
      if (bridge) {
        const controller = new AgentLifecycleController(bridge, world.agent);
        setAgentController(controller);
        setAgentLifecycle(controller.snapshot());
        setActiveBridge(bridge);
        setLoadError(null);
        return;
      }
      setLoadError(
        'An injected world must include the bridge that answers its item actions.',
      );
      return;
    }
    let alive = true;
    let bootInvalidated = false;
    let restartScheduled = false;
    let stopLifecycle: (() => void) | undefined;
    let stopMaintenance: (() => void) | undefined;
    const restartFromMaintenanceSnapshot = (): void => {
      bootInvalidated = true;
      if (restartScheduled) return;
      restartScheduled = true;
      setBootEpoch((value) => value + 1);
    };
    void (async () => {
      try {
        const selected = bridge ?? (await selectBridge());
        const controller = new AgentLifecycleController(selected);
        stopLifecycle = controller.subscribe((state) => {
          if (alive) setAgentLifecycle(state);
        });
        if (alive) setAgentController(controller);
        stopMaintenance = await selected.onMaintenanceStatus((snapshot) => {
          if (!alive) return;
          const accepted = controller.applyMaintenance(snapshot);
          // Until VaultShell owns ingestion, any native maintenance
          // transition invalidates this boot attempt. Restart from the
          // replayable snapshot so an in-flight pre-maintenance catalog can
          // never be published and consumed events are not lost at handoff.
          if (accepted && snapshot.state !== 'idle')
            restartFromMaintenanceSnapshot();
        });
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
        let next: World;
        try {
          next = await loadWorld(selected);
        } catch (error) {
          const typed = normalizeCommandError(error);
          if (
            !requested ||
            typed.code === 'bootstrap-required' ||
            typed.code === 'agent-lost' ||
            typed.code === 'version-mismatch' ||
            typed.fatal
          )
            throw error;
          next = emptyWorld(status);
        }
        if (!alive || bootInvalidated) return;
        // On an ordinary launch, look for teams that were granted to an
        // account after its first-run setup. An explicit first-run location
        // keeps its own discovery step, so leave it untouched.
        if (!requested && profileInventoryComplete(next, 'accounts')) {
          try {
            if (await discoverUnboundTeams(selected, next)) {
              if (!alive || bootInvalidated) return;
              next = await loadWorld(selected);
            }
          } catch {
            // Team discovery is best-effort and must not block launch.
          }
        }
        if (!alive || bootInvalidated) return;
        if (
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
            setManagedProfile(localProfile);
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
        stopMaintenance?.();
        stopMaintenance = undefined;
        setLoaded(next);
        setLoadError(null);
      } catch (error) {
        if (!alive) return;
        setAgentLifecycle({ state: 'failure', error });
        setLoadError(normalizeCommandError(error).message);
      }
    })();
    return () => {
      alive = false;
      stopLifecycle?.();
      stopMaintenance?.();
    };
  }, [bootEpoch, bridge, world]);

  // Re-arm the app lock from the shell. The command layer arms the lock and
  // returns its state; on a platform that cannot authenticate, `locked` stays
  // false and nothing is torn down.
  const lockNow = useCallback(async (): Promise<boolean> => {
    if (!activeBridge) return false;
    const next = await activeBridge.lockApp();
    if (!next.locked) return false;
    setLoaded(null);
    setLockState(next);
    return true;
  }, [activeBridge]);

  if (lockState && activeBridge) {
    const mechanism =
      lockState.mechanism === 'biometry'
        ? 'Touch ID or your Mac password'
        : 'your operating-system password';
    return (
      <div
        className="app-lock"
        role="dialog"
        aria-modal="true"
        aria-labelledby="app-lock-title"
      >
        <div className="app-lock-card">
          <h1 id="app-lock-title">Unlock FOKS</h1>
          <p>
            Authenticate with {mechanism} to allow FOKS to connect to servers
            and read vault data.
          </p>
          {lockError ? (
            <p className="app-lock-error" role="alert">
              {lockError}
            </p>
          ) : null}
          <Button
            variant="primary"
            disabled={unlocking}
            onClick={() => {
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
            }}
          >
            Unlock
          </Button>
        </div>
      </div>
    );
  }

  if (loadError) {
    return (
      <div
        className="app-lock"
        role="alertdialog"
        aria-labelledby="app-boot-error-title"
      >
        <div className="app-lock-card">
          <h1 id="app-boot-error-title">Couldn’t load FOKS</h1>
          <p className="app-lock-error" role="alert">
            {loadError}
          </p>
          <Button
            variant="primary"
            onClick={() => {
              setLoadError(null);
              setLoaded(null);
              setActiveBridge(null);
              setBootEpoch((value) => value + 1);
            }}
          >
            Retry
          </Button>
        </div>
      </div>
    );
  }
  if (!loaded || !activeBridge || !agentController) {
    if (
      agentLifecycle.state === 'recovery-required' ||
      agentLifecycle.state === 'restart-required' ||
      agentLifecycle.state === 'restoration-failed'
    ) {
      const recovery = agentLifecycle.state === 'recovery-required';
      const restoration = agentLifecycle.state === 'restoration-failed';
      return (
        <div className="app-lock" role="alertdialog">
          <div className="app-lock-card">
            <h1>{agentLifecycleLabel(agentLifecycle)}</h1>
            <p>{maintenanceOutcomeMessage(agentLifecycle.operation)}</p>
            <p>
              {recovery
                ? `Client state at ${agentLifecycle.root} needs explicit recovery. Run the supported “state status” and “state recover” CLI commands before reopening FOKS.`
                : restoration
                  ? agentLifecycle.error.message
                  : `FOKS must restart before it can open ${agentLifecycle.root}.`}
            </p>
            {restoration ? (
              <Button
                variant="primary"
                onClick={() => {
                  void agentController
                    ?.establish(true)
                    .then(() => setBootEpoch((value) => value + 1))
                    .catch((error) => {
                      // A successful native restoration publishes a newer
                      // maintenance snapshot while retryAgentConnection is
                      // still awaited. That transition intentionally makes
                      // this older establish attempt stale; the startup
                      // listener above owns the boot continuation.
                      const current = agentController.snapshot();
                      if (
                        current.state === 'checking' ||
                        current.state === 'ready'
                      )
                        return;
                      setLoadError(normalizeCommandError(error).message);
                    });
                }}
              >
                Retry service restart
              </Button>
            ) : null}
          </div>
        </div>
      );
    }
    return (
      <div className="app-loading">
        {agentLifecycle.state === 'checking'
          ? 'Connecting to the local agent…'
          : `${agentLifecycleLabel(agentLifecycle)}…`}
      </div>
    );
  }
  return (
    <VaultShell
      world={loaded}
      bridge={activeBridge}
      store={store}
      firstRunStart={firstRunStart}
      managedProfile={managedProfile}
      onLock={lockNow}
      agentController={agentController}
      leaseClock={leaseClock}
    />
  );
}

interface VaultShellProps {
  world: World;
  bridge: Bridge;
  store?: LocationStore;
  firstRunStart?: 'who' | 'local' | null;
  managedProfile?: string | null;
  onLock: () => Promise<boolean>;
  agentController: AgentLifecycleController;
  leaseClock?: LeaseExpiryClock;
}

function VaultShell({
  world,
  bridge,
  store,
  firstRunStart = null,
  managedProfile = null,
  onLock,
  agentController,
  leaseClock = systemLeaseExpiryClock,
}: VaultShellProps): ReactNode {
  const [{ scene, automaticFirstRun }] = useState(() => {
    const decoded = initialScene();
    const automatic = Boolean(
      firstRunStart && decoded.location.kind !== 'first-run',
    );
    return {
      scene:
        automatic && firstRunStart
          ? {
              ...decoded,
              location: { kind: 'first-run' as const, step: firstRunStart },
            }
          : decoded,
      automaticFirstRun: automatic,
    };
  });
  const [initialSelection] = useState(
    () => scene.selection ?? demoSelection(scene.demo, world),
  );
  const [fallback] = useState(() => {
    return storeAtScene({ ...scene, selection: initialSelection });
  });
  const locations = store ?? fallback;
  const navigateFromNotification = useCallback(
    (location: Parameters<LocationStore['navigate']>[0]) =>
      locations.navigate(location),
    [locations],
  );
  const state = useLocationState(locations);
  const [latest, setLatest] = useState(world);
  const latestRef = useRef(latest);
  latestRef.current = latest;
  const [observedExpiredLeases, setObservedExpiredLeases] = useState(
    world.observedExpiredLeases,
  );
  const [agentLifecycle, setAgentLifecycle] = useState<AgentLifecycle>(() =>
    agentController.snapshot(),
  );
  const [agentCatalogReady, setAgentCatalogReady] = useState(true);
  const [refreshingWorld, setRefreshingWorld] = useState(false);
  const [workflow, setWorkflow] = useState<WriteWorkflow>(() =>
    initialWriteWorkflow(
      typeof window === 'undefined' ? '' : window.location.search,
      world,
    ),
  );
  const [toasts] = useState(() => new ToastController());
  const [concealSignal, setConcealSignal] = useState(0);
  const [accessGenerations, setAccessGenerations] = useState<
    ReadonlyMap<string, number>
  >(() => new Map());
  const accessSession = useRef<object>({}).current;
  const [windowChromeHidden, setWindowChromeHidden] = useState(false);
  const [resumeDraft, setResumeDraft] = useState<{
    store: string;
    path: string;
    value: string;
    epoch: number;
  } | null>(null);
  const [revealRequest, setRevealRequest] = useState<string | null>(() =>
    scene.reveal && initialSelection
      ? `${initialSelection.store}|${initialSelection.path}`
      : null,
  );
  // Captured once because the canonical URL rewrite removes fixture-only intent.
  const [namedState] = useState(() =>
    typeof window === 'undefined'
      ? ''
      : (new URLSearchParams(window.location.search).get('state') ?? ''),
  );

  // Apply URL query lease overrides to the active world snapshot.
  const shown = useMemo(() => {
    const reconciled = { ...latest, observedExpiredLeases };
    const leased =
      scene.lease === 'lapsed' ? applyLease(reconciled, 'lapsed') : reconciled;
    const demonstrated = demoAvailabilityFacts(leased, namedState);
    if (
      bridge.firstRunFixture &&
      state.location.kind === 'first-run' &&
      state.location.step === 'boot'
    ) {
      return {
        ...demonstrated,
        agent: { state: 'bootstrap' as const, step: 'create-state' },
      };
    }
    return demonstrated;
  }, [
    bridge.firstRunFixture,
    latest,
    namedState,
    observedExpiredLeases,
    scene.lease,
    state.location,
  ]);

  useEffect(() => setLatest(world), [world]);

  useEffect(() => {
    if (!bridge.native) return;
    let disposed = false;
    let stop: (() => void) | undefined;
    void bridge
      .onOpenSettings(() => locations.navigate({ kind: 'settings' }))
      .then((unlisten) => {
        if (disposed) {
          unlisten();
          return;
        }
        stop = unlisten;
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      stop?.();
    };
  }, [bridge, locations]);

  useEffect(() => {
    if (!bridge.native) return;
    let disposed = false;
    let stop: (() => void) | undefined;
    const apply = ({
      maximized,
      fullscreen,
    }: Awaited<ReturnType<Bridge['windowState']>>): void => {
      if (!disposed) setWindowChromeHidden(maximized || fullscreen);
    };
    void bridge
      .onWindowState(apply)
      .then(async (unlisten) => {
        if (disposed) {
          unlisten();
          return;
        }
        stop = unlisten;
        apply(await bridge.windowState());
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      stop?.();
    };
  }, [bridge]);

  // Catalog loads cancel the previous generation. Ordinary callers share one
  // in-flight load; a mutation passes `force` because it invalidated any load
  // that was already running against the old catalog.
  const refreshWorldInFlight = useRef<Promise<World> | null>(null);
  const refreshWorldGeneration = useRef(0);
  const refreshWorld = useCallback(
    (force = false): Promise<World> => {
      if (force) refreshWorldInFlight.current = null;
      if (!refreshWorldInFlight.current) {
        const generation = ++refreshWorldGeneration.current;
        const pending = loadWorld(bridge, latestRef.current)
          .then((next) => {
            if (generation === refreshWorldGeneration.current) {
              setLatest(next);
              setAgentCatalogReady(true);
            }
            return next;
          })
          .finally(() => {
            if (refreshWorldInFlight.current === pending)
              refreshWorldInFlight.current = null;
          });
        refreshWorldInFlight.current = pending;
      }
      return refreshWorldInFlight.current;
    },
    [bridge],
  );

  const refresh = useCallback(
    async (message: string): Promise<void> => {
      await refreshWorld(true);
      toasts.show(message);
    },
    [refreshWorld, toasts],
  );

  const refreshWorldRef = useRef(refreshWorld);
  const commandErrorRef = useRef<(error: unknown) => void>(() => undefined);
  const foregroundRefreshAllowed = useRef(false);
  refreshWorldRef.current = refreshWorld;
  foregroundRefreshAllowed.current =
    latest.agent.state === 'ready' &&
    agentController.snapshot().state === 'ready' &&
    agentCatalogReady;

  const recoverAgentReadiness = useCallback(
    async (reconnect: boolean): Promise<void> => {
      try {
        await agentController.establish(reconnect);
      } catch (error) {
        // Native restoration success publishes its maintenance transition
        // before retryAgentConnection resolves. That newer transition makes
        // this establish attempt stale; its event handler owns the follow-up
        // establish and catalog refresh. Real retry failures publish failure.
        if (reconnect && agentController.snapshot().state !== 'failure') return;
        throw error;
      }
      // Readiness and catalog availability are separate. A catalog failure
      // leaves the connected agent ready and is reported by the caller.
      await refreshWorld(true);
    },
    [agentController, refreshWorld],
  );

  const commandError = useCallback(
    (error: unknown, item?: Item, draft = ''): void => {
      const typed = normalizeCommandError(error);
      const routed = workflowForError(error, item, draft);
      if (routed) {
        if (routed.kind === 'agent-lost')
          setConcealSignal((value) => value + 1);
        setWorkflow(routed);
        return;
      }
      // Catalog synchronization requests use standard toast notifications rather than warning alerts.
      toasts.show(
        typed.message,
        typed.code === 'catalog-required' ? undefined : { tone: 'warning' },
      );
    },
    [toasts],
  );
  commandErrorRef.current = commandError;

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
          void refreshWorldRef.current(true).catch((error: unknown) =>
            commandErrorRef.current(error),
          );
      },
    );
    expiryCoordinator.current = coordinator;
    const reconcileForeground = (): void => {
      coordinator.foreground();
      if (foregroundRefreshAllowed.current)
        void refreshWorldRef.current().catch((error: unknown) =>
          commandErrorRef.current(error),
        );
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
  }, [leaseClock]);

  useEffect(() => {
    expiryCoordinator.current?.update(
      latest.servers,
      latest.observedExpiredLeases,
    );
  }, [latest.observedExpiredLeases, latest.servers]);

  const mutationError = useCallback<MutationFailureHandler>(
    async (error, options = {}) => {
      const typed = normalizeCommandError(error);
      if (options.report !== false || typed.code === 'agent-lost')
        commandError(error, options.item, options.draft);
      await reconcileMutationFailure(
        error,
        () =>
          refresh(
            options.report === false
              ? 'Vault refreshed'
              : 'Vault refreshed. Try again.',
          ),
        commandError,
      );
    },
    [commandError, refresh],
  );

  const refreshAll = (): void => {
    if (refreshingWorld) return;
    setRefreshingWorld(true);
    void refreshWorld()
      .then(() => toasts.show('Vaults and groups refreshed'))
      .catch(commandError)
      .finally(() => setRefreshingWorld(false));
  };

  useEffect(() => {
    return agentController.subscribe(setAgentLifecycle);
  }, [agentController]);

  useEffect(() => {
    if (!bridge.native) return;
    let alive = true;
    let stop: (() => void) | undefined;
    const apply = (snapshot: Awaited<ReturnType<Bridge['clientStateMaintenanceStatus']>>): void => {
      if (!alive) return;
      if (!agentController.applyMaintenance(snapshot)) return;
      if (snapshot.state === 'idle') return;
      foregroundRefreshAllowed.current = false;
      refreshWorldGeneration.current++;
      refreshWorldInFlight.current = null;
      setAgentCatalogReady(false);
      setConcealSignal((value) => value + 1);
      if (snapshot.state !== 'complete') return;
      if (snapshot.operation.status === 'failed')
        commandError(snapshot.operation.error);
      else if (snapshot.operation.status === 'completed')
        toasts.show(maintenanceOutcomeMessage(snapshot.operation));
      if (snapshot.disposition.status === 'restoration-failed')
        commandError(snapshot.disposition.error);
      if (snapshot.disposition.status === 'continue-current-root')
        void agentController
          .establish(false)
          .then(() => refreshWorld(true))
          .catch(commandError);
    };
    void bridge
      .onMaintenanceStatus(apply)
      .then(async (unlisten) => {
        if (!alive) {
          unlisten();
          return;
        }
        stop = unlisten;
        apply(await bridge.clientStateMaintenanceStatus());
      })
      .catch(commandError);
    return () => {
      alive = false;
      stop?.();
    };
  }, [agentController, bridge, commandError, refreshWorld, toasts]);

  const handledReadinessErrors = useRef(new WeakSet<CommandError>());
  const handleAgentReadinessFailure = useCallback(
    (error: CommandError) => {
      if (
        !isAgentReadinessError(error) ||
        handledReadinessErrors.current.has(error)
      )
        return;
      // checked() reports before rejecting with this same normalized object.
      // A component forwarding that rejection must not invalidate recovery twice.
      handledReadinessErrors.current.add(error);
      foregroundRefreshAllowed.current = false;
      refreshWorldGeneration.current++;
      refreshWorldInFlight.current = null;
      setAgentCatalogReady(false);
      if (error.code === 'agent-lost') {
        agentController.disconnect(error.message);
        setConcealSignal((value) => value + 1);
        setWorkflow({ kind: 'agent-lost', message: error.message });
        return;
      }
      if (error.code === 'version-mismatch') {
        agentController.fail(error);
        commandError(error);
        return;
      }
      const step = error.details?.reason ?? 'initialize-state';
      agentController.requireBootstrap(step);
      // Onboarding owns an explicit Retry setup action. Keep automatic
      // recovery for commands issued elsewhere in the shell.
      if (state.location.kind !== 'first-run')
        void recoverAgentReadiness(false).catch(commandError);
    },
    [agentController, commandError, recoverAgentReadiness, state.location.kind],
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
        const message = await bridge.takeAgentConnectionLoss();
        if (alive && message) {
          foregroundRefreshAllowed.current = false;
          refreshWorldGeneration.current++;
          refreshWorldInFlight.current = null;
          setAgentCatalogReady(false);
          agentController.disconnect(message);
          setConcealSignal((value) => value + 1);
          setWorkflow({ kind: 'agent-lost', message });
        }
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
  }, [agentController, bridge, commandError]);

  const appRef = useRef<HTMLDivElement>(null);
  const portalRoot = useMemo(
    () =>
      typeof document === 'undefined'
        ? null
        : (document.getElementById('overlays') ?? document.body),
    [],
  );
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

  // Persist the current scene in the URL so a reload restores it. Only values
  // that differ from the default are written.
  useEffect(() => {
    if (typeof window === 'undefined') return;
    const href = sceneHref(window.location.href, sceneOf(state, scene.lease));
    if (href === window.location.href) return;
    try {
      window.history.replaceState(null, '', href);
    } catch {
      // Non-navigable environments (e.g. file:// or test harnesses) retain state in memory.
    }
  }, [state, scene.lease]);

  // Escape clears search query first, then deselects active item.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key !== 'Escape') return;
      // Ignore Escape events already handled by open modal dialogs.
      if (event.defaultPrevented) return;
      const current = locations.getSnapshot();
      if (current.query) locations.search('');
      else if (current.selection) locations.select(null);
    };
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [locations]);

  const here = state.location;
  const pendingFirstRun = incompleteFirstRunCheckpoint();
  const detailsShown =
    state.details &&
    listsItems(here) &&
    !(here.kind === 'store' && !storeReadable(shown, here.ref)) &&
    !(state.selection && !storeReadable(shown, state.selection.store));
  const selectedAccessGeneration = state.selection
    ? accessGenerations.get(
        storeOf(shown, state.selection.store)?.server ?? '',
      ) ?? 0
    : 0;
  const screen = listsItems(here) ? (
    <ItemsScreen
      world={shown}
      bridge={bridge}
      state={state}
      locations={locations}
      onReveal={(item) => setRevealRequest(`${item.store}|${item.path}`)}
      onNew={(itemKind, storeId, initialFolder) =>
        setWorkflow({ kind: 'new', itemKind, storeId, initialFolder })
      }
      onResume={async (storeId) => {
        try {
          await bridge.resumeGroupCreation(storeId);
          await refresh('Group creation resumed');
        } catch (error) {
          await mutationError(error);
        }
      }}
      onDelete={(item) => setWorkflow({ kind: 'delete', item })}
      onSettings={(storeId) =>
        locations.navigate({
          kind: 'group-settings',
          ref: storeId,
          tab: 'people',
        })
      }
      onCommandError={commandError}
      accessNow={accessNow}
    />
  ) : here.kind === 'team-chat' ? (
    <ChatScreen
      key={`chat:${here.ref}:${concealSignal}`}
      world={shown}
      bridge={bridge}
      location={here}
      accessNow={accessNow}
      accessGeneration={
        accessGenerations.get(storeOf(shown, here.ref)?.server ?? '') ?? 0
      }
      onNavigate={(location) => locations.navigate(location)}
    />
  ) : here.kind === 'group-settings' ? (
    <GroupSettingsScreen
      key={`${here.kind}:${here.ref}`}
      world={shown}
      bridge={bridge}
      location={here}
      onNavigate={(location) => locations.navigate(location)}
      onApplied={refresh}
      onError={commandError}
      onMutationError={mutationError}
    />
  ) : here.kind === 'alerts' ? (
    <AlertsScreen
      world={shown}
      onRefreshWorld={refreshWorld}
      onError={commandError}
    />
  ) : here.kind === 'settings' ? (
    <SettingsScreen
      key={`settings:${concealSignal}`}
      world={shown}
      bridge={bridge}
      location={here}
      scene={namedState}
      onNavigate={(location) => locations.navigate(location)}
      onRefresh={refresh}
      onRefreshWorld={refreshWorld}
      onError={commandError}
      onMutationError={mutationError}
      onLock={onLock}
      agentLifecycle={agentLifecycle}
      onRetryAgent={() => recoverAgentReadiness(true)}
    />
  ) : (
    <PlaceholderScreen location={here} />
  );

  const shell = (
    <div
      className={[
        'window',
        bridge.native ? 'native-window' : 'web-mock-window',
        windowChromeHidden ? 'window-chrome-hidden' : '',
      ]
        .filter(Boolean)
        .join(' ')}
    >
      <div className="titlebar" data-tauri-drag-region="">
        {bridge.native ? null : (
          <span className="lights" aria-hidden="true">
            <span className="light r" />
            <span className="light y" />
            <span className="light g" />
          </span>
        )}
        <span className="brand" data-tauri-drag-region="">
          {APP_NAME}
        </span>
        <span className="spacer" data-tauri-drag-region="" />
        <span
          className={
            shown.agent.state === 'ready' && agentLifecycle.state === 'ready'
              ? 'agent'
              : 'agent warn'
          }
          data-tauri-drag-region=""
        >
          <i data-tauri-drag-region="" />
          {/* Local agent connection and readiness status */}
          Agent{' '}
          {shown.agent.state === 'bootstrap'
            ? 'starting'
            : agentLifecycle.state === 'ready'
              ? 'ready'
              : agentLifecycleLabel(agentLifecycle).toLowerCase()}
        </span>
        <Button
          variant="quiet"
          className="global-refresh"
          icon="again"
          aria-label={
            refreshingWorld ? 'Refreshing vaults and groups' : 'Refresh'
          }
          title={
            refreshingWorld
              ? 'Refreshing vaults and groups'
              : 'Refresh vaults and groups'
          }
          disabled={refreshingWorld}
          onClick={refreshAll}
        />
      </div>
      <div
        className={
          detailsShown || (here.kind === 'first-run' && here.step === 'added')
            ? 'app with-details'
            : 'app'
        }
        ref={appRef}
      >
        {here.kind === 'first-run' ? (
          <FirstRunExperience
            world={shown}
            bridge={bridge}
            location={here}
            onNavigate={(location) => locations.navigate(location)}
            onRefreshWorld={refreshWorld}
            concealSignal={concealSignal}
            agentReady={
              shown.agent.state === 'ready' &&
              agentLifecycle.state === 'ready' &&
              agentCatalogReady
            }
            onRetryAgent={() => recoverAgentReadiness(false)}
            onAgentReadinessFailure={handleAgentReadinessFailure}
            automaticEntry={automaticFirstRun}
            managedProfile={managedProfile ?? undefined}
          />
        ) : (
          <>
            <Sidebar
              world={shown}
              location={here}
              alerts={notesNow(shown).length}
              onNavigate={(location) => {
                locations.navigate(location);
              }}
              status={
                pendingFirstRun ? (
                  <FirstRunChecklistStatus
                    checkpoint={pendingFirstRun}
                    onNavigate={(location) => locations.navigate(location)}
                  />
                ) : undefined
              }
            />
            <main className="main">{screen}</main>
          </>
        )}
        {here.kind !== 'first-run' && detailsShown ? (
          <DetailsPanel
            world={shown}
            bridge={bridge}
            revealRequest={revealRequest}
            onRevealHandled={() => setRevealRequest(null)}
            selection={state.selection}
            onSelect={(selection) => {
              locations.select(selection);
            }}
            onClose={() => {
              locations.setDetails(false);
            }}
            onDelete={(item) => setWorkflow({ kind: 'delete', item })}
            onConflict={(item, draft) =>
              setWorkflow({ kind: 'conflict', item, draft })
            }
            onApplied={refresh}
            onCommandError={commandError}
            onMutationError={mutationError}
            concealSignal={concealSignal}
            accessGeneration={selectedAccessGeneration}
            accessNow={accessNow}
            accessSession={accessSession}
            resumeDraft={resumeDraft}
          />
        ) : null}
      </div>
      {agentLifecycle.state === 'maintenance' ||
      agentLifecycle.state === 'restart-required' ||
      agentLifecycle.state === 'recovery-required' ||
      agentLifecycle.state === 'restoration-failed' ? (
        <Dialog
          className="stopwrap"
          role="alertdialog"
          aria-label={agentLifecycleLabel(agentLifecycle)}
        >
          <div className="notice stop">
            <h2>{agentLifecycleLabel(agentLifecycle)}</h2>
            {agentLifecycle.state !== 'maintenance' ? (
              <p>{maintenanceOutcomeMessage(agentLifecycle.operation)}</p>
            ) : null}
            <p>
              {agentLifecycle.state === 'maintenance'
                ? 'FOKS has paused local-agent access while protected state maintenance finishes.'
                : agentLifecycle.state === 'recovery-required'
                  ? `Client state at ${agentLifecycle.root} needs explicit recovery. Run the supported “state status” and “state recover” CLI commands before reopening FOKS.`
                  : agentLifecycle.state === 'restart-required'
                    ? `FOKS must restart before it can open ${agentLifecycle.root}.`
                    : agentLifecycle.error.message}
            </p>
            {agentLifecycle.state === 'restoration-failed' ? (
              <div className="acts2">
                <Button
                  variant="primary"
                  onClick={() => {
                    void recoverAgentReadiness(true).catch(commandError);
                  }}
                >
                  Retry service restart
                </Button>
              </div>
            ) : null}
          </div>
        </Dialog>
      ) : null}
      <WriteOverlay
        world={shown}
        accessNow={accessNow}
        bridge={bridge}
        workflow={workflow}
        setWorkflow={setWorkflow}
        onApplied={refresh}
        onError={commandError}
        onMutationError={mutationError}
        onRetryAgent={() => recoverAgentReadiness(true)}
        onRefreshConflict={async (item, draft) => {
          await refreshWorld();
          setResumeDraft({
            store: item.store,
            path: item.path,
            value: draft,
            epoch: Date.now(),
          });
          toasts.show('Catalog refreshed. Review your draft.');
        }}
        onDiscardConflict={() => {
          setWorkflow(null);
          setResumeDraft(null);
          setConcealSignal((value) => value + 1);
        }}
        onOpenExisting={async (existing) => {
          const next = await refreshWorld();
          const current = next.items.find(
            (item) =>
              item.store === existing.storeId && item.path === existing.path,
          );
          if (!current) {
            throw new Error(
              'That path is now available. Your draft has been saved. Choose a different path or retry creating the item.',
            );
          }
          setLatest(next);
          locations.select({ store: current.store, path: current.path });
        }}
      />
    </div>
  );

  const withToasts = (
    <ToastProvider controller={toasts} portalRoot={portalRoot}>
      <ChatInboxProvider
        key={`inbox:${concealSignal}`}
        bridge={bridge}
        world={shown}
        onNavigate={navigateFromNotification}
        clock={chatClock}
      >
        {shell}
      </ChatInboxProvider>
    </ToastProvider>
  );
  if (!portalRoot) return withToasts;
  return (
    <OverlayProvider backgroundRef={appRef} portalRoot={portalRoot}>
      {withToasts}
    </OverlayProvider>
  );
}

function emptyWorld(agent: World['agent']): World {
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

/** Applies review-scene failures once at the fixture/model boundary. */
function demoAvailabilityFacts(world: World, state: string): World {
  if (
    state === 'servers-list' ||
    state === 'servers-add' ||
    state === 'servers-lapsed'
  )
    return applyLease(world, 'lapsed');
  if (state === 'servers-rollback') {
    const error = {
      code: 'host-verification-failed',
      message: 'The fixture host identity moved backwards.',
      fatal: false,
      retryable: false,
      ambiguous: false,
    };
    return {
      ...world,
      servers: world.servers.map((server) =>
        server.id === 'personal'
          ? { ...server, trust: { status: 'blocked' as const, error } }
          : server,
      ),
    };
  }
  if (state === 'settings-account') {
    const error = {
      code: 'catalog-unavailable',
      message: 'The fixture vault inventory is unavailable.',
      fatal: false,
      retryable: true,
      ambiguous: false,
    };
    return {
      ...world,
      storeInventory: world.storeInventory.map((entry) =>
        entry.store === 'acct:work'
          ? { ...entry, status: 'unavailable' as const, error }
          : entry,
      ),
    };
  }
  return world;
}

function demoSelection(
  demo: Scene['demo'],
  world: World,
): { store: string; path: string } | null {
  if (!demo) return null;
  const candidates = world.items.filter((item) => item.kind !== 'Folder');
  let item: Item | undefined;
  if (demo === 'password')
    item = candidates.find((candidate) => isLogin(candidate));
  else if (demo === 'resource') {
    item = candidates
      .filter(
        (candidate) =>
          kindOf(candidate) === 'Resource' &&
          storeOf(world, candidate.store)?.kind === 'account',
      )
      .sort((left, right) =>
        nameOf(left.path).localeCompare(nameOf(right.path)),
      )[0];
  } else if (demo === 'file') {
    item = candidates
      .filter(
        (candidate) =>
          kindOf(candidate) === 'File' &&
          storeOf(world, candidate.store)?.kind === 'team',
      )
      .sort((left, right) => right.version - left.version)[0];
  } else if (demo === 'link')
    item = candidates.find((candidate) => kindOf(candidate) === 'Link');
  else {
    item = candidates
      .filter(
        (candidate) =>
          kindOf(candidate) === 'Password' &&
          storeOf(world, candidate.store)?.kind === 'team',
      )
      .sort((left, right) =>
        nameOf(left.path).localeCompare(nameOf(right.path)),
      )[0];
  }
  return item ? { store: item.store, path: item.path } : null;
}
