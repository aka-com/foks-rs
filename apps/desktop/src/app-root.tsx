/**
 * Root application component for the FOKS desktop vault shell.
 *
 * Integrates sidebar navigation, item screens, details panels, and modal
 * workflows with bridge IPC and URL location state.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { OverlayProvider } from '/kit/overlay-primitives';
import { ToastController, ToastProvider } from '/kit/toasts';
import {
  discoverUnboundTeams,
  loadWorld,
  normalizeCommandError,
  selectBridge,
} from './bridge';
import type { AppLockState, Bridge } from './bridge';
import { Button } from './components';
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
import { GroupSettingsScreen } from './screens/groups-screen';
import { FirstRunExperience } from './screens/first-run-screen';
import { PlaceholderScreen } from './screens/placeholder-screen';
import { SettingsScreen } from './screens/settings-screen';
import { listsItems } from './screens/scope';
import {
  initialWriteWorkflow,
  workflowForError,
  WriteOverlay,
} from './screens/write-workflows';
import type { WriteWorkflow } from './screens/write-workflows';

/** The app's name, as the title bar and the first-run sidebar write it. */
export const APP_NAME = 'FOKS';

/** The scene the address bar asks for, or the shell's own starting point. */
function initialScene(): Scene {
  if (typeof window === 'undefined') return INITIAL_SCENE;
  return decodeScene(window.location.search);
}

export interface AppProps {
  /** Supplying a world makes render tests synchronous. Production omits it. */
  world?: World;
  /** A command seam for tests; production selects the Tauri or mock bridge. */
  bridge?: Bridge;
  /** Injected by the render tests so navigation is observable. */
  store?: LocationStore;
}

export function App({ world, bridge, store }: AppProps): ReactNode {
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

  useEffect(() => {
    if (world) {
      setLoaded(world);
      if (bridge) {
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
    void (async () => {
      try {
        const selected = bridge ?? (await selectBridge());
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
        const requested = initialScene().location.kind === 'first-run';
        const [status, appInfo] = await Promise.all([
          selected.agentStatus(),
          selected.appInfo(),
        ]);
        if (alive) setManagedProfile(appInfo.managedProfile ?? null);
        let next: World;
        if (status.phase !== 'Ready') {
          next = emptyWorld(status);
          if (alive) setFirstRunStart('who');
        } else {
          try {
            next = await loadWorld(selected);
          } catch (error) {
            if (!requested) throw error;
            next = emptyWorld(status);
          }
          // On an ordinary launch, look for teams that were granted to an
          // account after its first-run setup. An explicit first-run location
          // keeps its own discovery step, so leave it untouched.
          if (!requested && next.accountInventoryComplete) {
            try {
              if (await discoverUnboundTeams(selected, next))
                next = await loadWorld(selected);
            } catch {
              // Team discovery is best-effort and must not block launch.
            }
          }
          if (
            !requested &&
            next.accountInventoryComplete &&
            !next.stores.some((entry) => entry.kind === 'account')
          ) {
            const localProfile =
              appInfo.managedProfile &&
              next.servers.some(
                (server) =>
                  server.id === appInfo.managedProfile && server.state === 'ok',
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
        }
        if (!alive) return;
        setLoaded(next);
        setLoadError(null);
      } catch (error) {
        if (!alive) return;
        setLoadError(normalizeCommandError(error).message);
      }
    })();
    return () => {
      alive = false;
    };
  }, [bootEpoch, bridge, world]);

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
  if (!loaded || !activeBridge) {
    return <div className="app-loading">Connecting to the local agent…</div>;
  }
  return (
    <VaultShell
      world={loaded}
      bridge={activeBridge}
      store={store}
      firstRunStart={firstRunStart}
      managedProfile={managedProfile}
    />
  );
}

interface VaultShellProps {
  world: World;
  bridge: Bridge;
  store?: LocationStore;
  firstRunStart?: 'who' | 'local' | null;
  managedProfile?: string | null;
}

function VaultShell({
  world,
  bridge,
  store,
  firstRunStart = null,
  managedProfile = null,
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
  const state = useLocationState(locations);
  const [latest, setLatest] = useState(world);
  const [refreshingWorld, setRefreshingWorld] = useState(false);
  const [workflow, setWorkflow] = useState<WriteWorkflow>(() =>
    initialWriteWorkflow(
      typeof window === 'undefined' ? '' : window.location.search,
      world,
    ),
  );
  const [toasts] = useState(() => new ToastController());
  const [concealSignal, setConcealSignal] = useState(0);
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

  // Apply URL query lease overrides to the active world snapshot.
  const shown = useMemo(() => {
    const leased =
      scene.lease === 'lapsed' ? applyLease(latest, 'lapsed') : latest;
    if (
      bridge.firstRunFixture &&
      state.location.kind === 'first-run' &&
      state.location.step === 'boot'
    ) {
      return {
        ...leased,
        agent: { phase: 'Bootstrap' as const, step: 'create-state' },
      };
    }
    return leased;
  }, [bridge.firstRunFixture, latest, scene.lease, state.location]);

  useEffect(() => setLatest(world), [world]);

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
        const pending = loadWorld(bridge)
          .then((next) => {
            if (generation === refreshWorldGeneration.current) setLatest(next);
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
    let alive = true;
    let pending = false;
    const check = async (): Promise<void> => {
      if (pending) return;
      pending = true;
      try {
        const message = await bridge.takeAgentConnectionLoss();
        if (alive && message) {
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
  }, [bridge, commandError]);

  const appRef = useRef<HTMLDivElement>(null);
  const portalRoot = useMemo(
    () =>
      typeof document === 'undefined'
        ? null
        : (document.getElementById('overlays') ?? document.body),
    [],
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
  // Captured once, the way SettingsScreen captures its
  // scene: the effect below rewrites the address bar to the canonical scene
  // on the first commit, so fixture-only sheet intent must retain its name.
  const [namedState] = useState(() =>
    typeof window === 'undefined'
      ? ''
      : (new URLSearchParams(window.location.search).get('state') ?? ''),
  );
  const detailsShown =
    state.details &&
    listsItems(here) &&
    !(here.kind === 'store' && !storeReadable(shown, here.ref)) &&
    !(state.selection && !storeReadable(shown, state.selection.store));
  const screen = listsItems(here) ? (
    <ItemsScreen
      world={shown}
      bridge={bridge}
      state={state}
      locations={locations}
      onReveal={(item) => setRevealRequest(`${item.store}|${item.path}`)}
      onNew={(itemKind, storeId) =>
        setWorkflow({ kind: 'new', itemKind, storeId })
      }
      onResume={async (storeId) => {
        try {
          await bridge.resumeGroupCreation(storeId);
          await refresh('Group creation resumed');
        } catch (error) {
          await mutationError(error);
        }
      }}
      onRemove={(item) => setWorkflow({ kind: 'remove', item })}
      onSettings={(storeId) =>
        locations.navigate({
          kind: 'group-settings',
          ref: storeId,
          tab: 'people',
        })
      }
      onCommandError={commandError}
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
          className={shown.agent.phase === 'Bootstrap' ? 'agent warn' : 'agent'}
          data-tauri-drag-region=""
        >
          <i data-tauri-drag-region="" />
          {/* Local agent connection and readiness status */}
          Agent {shown.agent.phase === 'Bootstrap' ? 'starting' : 'ready'}
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
            onRemove={(item) => setWorkflow({ kind: 'remove', item })}
            onConflict={(item, draft) =>
              setWorkflow({ kind: 'conflict', item, draft })
            }
            onApplied={refresh}
            onCommandError={commandError}
            onMutationError={mutationError}
            concealSignal={concealSignal}
            resumeDraft={resumeDraft}
          />
        ) : null}
      </div>
      <WriteOverlay
        world={shown}
        bridge={bridge}
        workflow={workflow}
        setWorkflow={setWorkflow}
        onApplied={refresh}
        onError={commandError}
        onMutationError={mutationError}
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
      {shell}
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
    unavailableStores: [],
    accountInventoryComplete: false,
    items: [],
    parties: [],
    federation: [],
    groupDetailFailures: [],
    devices: [],
    yubiAccounts: [],
    cardsConnected: [],
    notifications: [],
    leaseState: 'fresh',
    plaintext: {},
  };
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
