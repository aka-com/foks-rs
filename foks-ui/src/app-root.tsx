/**
 * The vault shell.
 *
 * The window, the sidebar, the item page and the details panel, with the
 * desktop fixture behind them. React owns the DOM, and
 * `tests/react-boundary.test.ts` forbids raw-HTML sinks here.
 *
 * First run is a resumable location in this same window. Servers and Settings
 * remain separate later surfaces. Values stay masked until Show performs the
 * version-bound read.
 *
 * Where the shell is lives in the location store, and the whole of it is a
 * deep link: `?state=`, plus `sel`, `view`, `kind`, `sort` and `lease`. See
 * `README.md` for the table.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { OverlayProvider } from '/kit/overlay-primitives';
import { loadWorld, normalizeCommandError, selectBridge } from './bridge';
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
import { applyLease, isLogin, kindOf, nameOf, notesNow, storeOf, storeReadable } from './model';
import type { Item, World } from './model';
import { Sidebar } from './shell/sidebar';
import { IssuesScreen } from './screens/issues-screen';
import { DetailsPanel } from './screens/details-panel';
import { ItemsScreen } from './screens/items-screen';
import { GroupsScreen, VaultGroupOverlay } from './screens/groups-screen';
import { FirstRunExperience } from './screens/first-run-screen';
import { PlaceholderScreen } from './screens/placeholder-screen';
import { ServersScreen } from './screens/servers-screen';
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
  const [loaded, setLoaded] = useState<World | null>(
    () => world ?? null,
  );
  const [loadError, setLoadError] = useState<string | null>(null);
  const [firstRunStart, setFirstRunStart] = useState<'boot' | 'who' | 'local' | null>(null);
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
      setLoadError('An injected world must include the bridge that answers its item actions.');
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
          if (alive) setFirstRunStart('boot');
        } else {
          try {
            next = await loadWorld(selected);
          } catch (error) {
            if (!requested) throw error;
            next = emptyWorld(status);
          }
          if (!requested && !next.stores.some((entry) => entry.kind === 'account')) {
            const localProfile = appInfo.managedProfile && next.servers.some(
              (server) => server.id === appInfo.managedProfile && server.state === 'ok',
            ) ? appInfo.managedProfile : null;
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
    const mechanism = lockState.mechanism === 'biometry'
      ? 'Touch ID or your Mac password'
      : 'your operating-system password';
    return (
      <div className="app-lock" role="dialog" aria-modal="true" aria-labelledby="app-lock-title">
        <div className="app-lock-card">
          <h1 id="app-lock-title">Unlock FOKS</h1>
          <p>Authenticate with {mechanism} to allow FOKS to connect to servers and read vault data.</p>
          {lockError ? <p className="app-lock-error" role="alert">{lockError}</p> : null}
          <Button
            variant="primary"
            disabled={unlocking}
            onClick={() => {
              setUnlocking(true);
              setLockError(null);
              void activeBridge.unlockApp().then(
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
              ).finally(() => setUnlocking(false));
            }}
          >
            Unlock
          </Button>
        </div>
      </div>
    );
  }

  if (loadError) {
    return <div className="app-loading" role="alert">{loadError}</div>;
  }
  if (!loaded || !activeBridge) {
    return <div className="app-loading">Connecting to the local agent…</div>;
  }
  return <VaultShell world={loaded} bridge={activeBridge} store={store} firstRunStart={firstRunStart} managedProfile={managedProfile} />;
}

interface VaultShellProps {
  world: World;
  bridge: Bridge;
  store?: LocationStore;
  firstRunStart?: 'boot' | 'who' | 'local' | null;
  managedProfile?: string | null;
}

function VaultShell({ world, bridge, store, firstRunStart = null, managedProfile = null }: VaultShellProps): ReactNode {
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
  const [workflow, setWorkflow] = useState<WriteWorkflow>(() =>
    initialWriteWorkflow(typeof window === 'undefined' ? '' : window.location.search, world),
  );
  const [flash, setFlash] = useState<string | null>(null);
  const [concealSignal, setConcealSignal] = useState(0);
  const [groupSheetIntent, setGroupSheetIntent] = useState<{ store: string; sheet: 'manage' } | null>(null);
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

  // A lapsed compatibility lease is a property of the world, not a place in
  // it, so the deep link applies it to the world rather than storing it.
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

  const refreshWorld = useCallback(async (): Promise<World> => {
    const next = await loadWorld(bridge);
    setLatest(next);
    return next;
  }, [bridge]);

  const refresh = async (message: string): Promise<void> => {
    await refreshWorld();
    setFlash(message);
  };

  const commandError = useCallback((error: unknown, item?: Item, draft = ''): void => {
    const typed = normalizeCommandError(error);
    const routed = workflowForError(error, item, draft);
    if (routed) {
      if (routed.kind === 'agent-lost') setConcealSignal((value) => value + 1);
      setWorkflow(routed);
      return;
    }
    setFlash(typed.message);
  }, []);

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
        if (alive && normalizeCommandError(error).code === 'agent-lost') commandError(error);
      } finally {
        pending = false;
      }
    };
    void check();
    const timer = window.setInterval(() => void check(), 1000);
    return () => { alive = false; window.clearInterval(timer); };
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
      // A realm with no navigable history (a file:// page, a test) simply
      // keeps the state in memory.
    }
  }, [state, scene.lease]);

  // Escape steps back out: the search first, then the selection.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key !== 'Escape') return;
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
  const namedState = typeof window === 'undefined'
    ? ''
    : new URLSearchParams(window.location.search).get('state') ?? '';
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
          commandError(error);
        }
      }}
      onRemove={(item) => setWorkflow({ kind: 'remove', item })}
      onManage={(storeId) => {
        setGroupSheetIntent({ store: storeId, sheet: 'manage' });
        locations.navigate({ kind: 'group-admin', ref: storeId });
      }}
      onCommandError={commandError}
    />
  ) : here.kind === 'join' || here.kind === 'groups' || here.kind === 'group-admin' ? (
    <GroupsScreen key={here.kind === 'group-admin' ? `${here.kind}:${here.ref}` : here.kind} world={shown} bridge={bridge} location={here} onNavigate={(location) => locations.navigate(location)} onApplied={refresh} onError={commandError} initialSheet={here.kind === 'group-admin' && groupSheetIntent?.store === here.ref ? groupSheetIntent.sheet : undefined} onIntentConsumed={() => setGroupSheetIntent(null)} />
  ) : here.kind === 'issues' ? (
    <IssuesScreen world={shown} />
  ) : here.kind === 'servers' ? (
    <ServersScreen
      world={shown}
      bridge={bridge}
      location={here}
      scene={namedState}
      onNavigate={(location) => locations.navigate(location)}
      onRefresh={refresh}
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
      onError={commandError}
    />
  ) : (
    <PlaceholderScreen location={here} />
  );

  const shell = (
    <div className={bridge.native ? 'window native-window' : 'window web-mock-window'}>
      <div className="titlebar">
        {bridge.native ? null : (
          <span className="lights" aria-hidden="true">
            <span className="light r" />
            <span className="light y" />
            <span className="light g" />
          </span>
        )}
        <span className="brand">{APP_NAME}</span>
        <span className="spacer" />
        <span
          className={shown.agent.phase === 'Bootstrap' ? 'agent warn' : 'agent'}
        >
          <i />
          {/* The agent's own word for where it is, as the design frames it. */}
          Agent {shown.agent.phase === 'Bootstrap' ? 'starting' : 'ready'}
        </span>
      </div>
      <div
        className={detailsShown || (here.kind === 'first-run' && here.step === 'added') ? 'app with-details' : 'app'}
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
        ) : <>
          <Sidebar
            world={shown}
            location={here}
            issues={notesNow(shown).length}
            onNavigate={(location) => {
              locations.navigate(location);
            }}
          />
          <main className="main">{screen}</main>
        </>}
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
            onConflict={(item, draft) => setWorkflow({ kind: 'conflict', item, draft })}
            onApplied={refresh}
            onCommandError={commandError}
            concealSignal={concealSignal}
            resumeDraft={resumeDraft}
          />
        ) : null}
      </div>
      {flash ? <div className="flash" aria-live="polite">{flash}</div> : null}
      <WriteOverlay
        world={shown}
        bridge={bridge}
        workflow={workflow}
        setWorkflow={setWorkflow}
        onApplied={refresh}
        onError={commandError}
        onRefreshConflict={async (item, draft) => {
          const next = await loadWorld(bridge);
          setLatest(next);
          setResumeDraft({ store: item.store, path: item.path, value: draft, epoch: Date.now() });
          setFlash('Refreshed the catalog — review your retained draft');
        }}
        onDiscardConflict={() => {
          setWorkflow(null);
          setResumeDraft(null);
          setConcealSignal((value) => value + 1);
        }}
        onOpenExisting={async (existing) => {
          const next = await loadWorld(bridge);
          const current = next.items.find(
            (item) => item.store === existing.storeId && item.path === existing.path,
          );
          if (!current) {
            throw new Error('That path is free now. Your draft is still here; change the path or try creating it again.');
          }
          setLatest(next);
          locations.select({ store: current.store, path: current.path });
        }}
      />
      {namedState === 'manage' || namedState === 'party-remove' ? (
        <VaultGroupOverlay
          world={shown}
          bridge={bridge}
          scene={namedState}
          onApplied={refresh}
          onError={commandError}
        />
      ) : null}
    </div>
  );

  if (!portalRoot) return shell;
  return (
    <OverlayProvider backgroundRef={appRef} portalRoot={portalRoot}>
      {shell}
    </OverlayProvider>
  );
}

function emptyWorld(agent: World['agent']): World {
  return {
    agent,
    servers: [],
    accounts: [],
    stores: [],
    items: [],
    parties: [],
    federation: [],
    devices: [],
    yubiAccounts: [],
    cardsConnected: [],
    notifications: [],
    leaseState: 'fresh',
    plaintext: {},
  };
}

function demoSelection(demo: Scene['demo'], world: World): { store: string; path: string } | null {
  if (!demo) return null;
  const candidates = world.items.filter((item) => item.kind !== 'Folder');
  let item: Item | undefined;
  if (demo === 'password') item = candidates.find((candidate) => isLogin(candidate));
  else if (demo === 'resource') {
    item = candidates
      .filter((candidate) => kindOf(candidate) === 'Resource' && storeOf(world, candidate.store)?.kind === 'account')
      .sort((left, right) => nameOf(left.path).localeCompare(nameOf(right.path)))[0];
  } else if (demo === 'file') {
    item = candidates
      .filter((candidate) => kindOf(candidate) === 'File' && storeOf(world, candidate.store)?.kind === 'team')
      .sort((left, right) => right.version - left.version)[0];
  } else if (demo === 'link') item = candidates.find((candidate) => kindOf(candidate) === 'Link');
  else {
    item = candidates
      .filter((candidate) => kindOf(candidate) === 'Password' && storeOf(world, candidate.store)?.kind === 'team')
      .sort((left, right) => nameOf(left.path).localeCompare(nameOf(right.path)))[0];
  }
  return item ? { store: item.store, path: item.path } : null;
}
