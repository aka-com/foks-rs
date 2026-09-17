import { QueryRepositoryContext } from './query-hooks';
import { readRecoveryFor } from './query-read-recovery';
import { CatalogCoordinator } from './catalog-coordinator';
import { synchronizeApplied } from './operation-outcome';
import { accountStopped } from './model';
import { DeviceCache, DeviceCacheContext } from './device-cache';
import { useTabSheetState } from './navigation-guard';
import type { DeviceLabel } from './model';
import { ChatInboxProvider } from './chat/inbox-provider';
/**
 * Root application component for the FOKS desktop vault shell.
 *
 * Integrates sidebar navigation, item screens, details panels, and modal
 * workflows with bridge IPC and URL location state.
 */

import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react';
import type { ReactNode } from 'react';
import {
  Dialog,
  OverlayProvider,
  anyDialogOpen,
  useHasOverlayProvider,
} from '/kit/overlay-primitives';
import { ToastController, ToastProvider } from '/kit/toasts';
import {
  discoverUnboundTeams,
  isAgentReadinessError,
  loadSnapshot,
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
import { Button, CopyBox, Icon } from './components';
import {
  FIRST_RUN_CHECKPOINT_KEY,
  completedFirstRunSteps,
  decodeFirstRunCheckpoint,
  firstRunStepCount,
} from './first-run-state';
import type { FirstRunCheckpoint } from './first-run-state';
import { FIRST_RUN_PROGRESS_EVENT } from './first-run-operations';
import {
  LocationStore,
  decodeScene,
  parentLocation,
  rememberChatLocation,
  sceneHref,
  sceneOf,
  storeAtScene,
  useLocationState,
} from './location';
import { INITIAL_SCENE } from './location';
import type { Location, NavigateOptions, Scene } from './location';
import {
  NavigationGuardProvider,
  NavigationPrompt,
  type NavigationPromptVerdict,
} from './navigation-guard';
import {
  applyLease,
  isLogin,
  kindOf,
  nameOf,
  profileInventoryComplete,
  serverAvailability,
  settingsAlertSummary,
  storeOf,
  storeReadable,
} from './model';
import type { Item, AgentSnapshot } from './model';
import { reconcileMutationFailure } from './mutation-recovery';
import type { MutationFailureHandler } from './mutation-recovery';
import { Sidebar, railAgentState } from './shell/sidebar';
import type { RailAgentState } from './shell/sidebar';
import { Topbar } from './shell/topbar';
import { SearchPalette, useSearchShortcut } from './shell/search-palette';
import type { SearchChannel } from './shell/search-palette';
import { mountSwipeBack } from './shell/swipe-back';
import { useSidebarInbox } from './chat/inbox-provider';
import {
  rememberSideCollapsed,
  storedSideCollapsedPref,
} from './sidebar-prefs';
import { applyRailColor, storedRailColor } from './rail-theme';
import { PeopleScreen, unroutedNotices } from './screens/people-screen';
import {
  deviceAlertRegistry,
  devicesAlertSummary,
} from './screens/device-alert';
import {
  teamRequestRegistry,
  teamRequestsBadge,
} from './screens/team-requests';
import { DevicesScreen } from './screens/devices-screen';
import { DetailsPanel } from './screens/details-panel';
import { ItemsScreen } from './screens/items-screen';
import type { DropUpload } from './screens/items-screen';
import { ChatTab } from './screens/chat-tab';
import { TeamsScreen } from './screens/teams-screen';
import { GroupSettingsScreen } from './screens/groups-screen';
import {
  FirstRunChecklistStatus,
  FirstRunExperience,
} from './screens/first-run-screen';
import { PlaceholderScreen } from './screens/placeholder-screen';
import { SettingsScreen } from './screens/settings-screen';
import { listsItems } from './screens/scope';
import {
  DEFAULT_READ_ROLE,
  DEFAULT_WRITE_ROLE,
  droppedFileDraft,
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

type AgentStop = Extract<
  AgentLifecycle,
  {
    state:
      | 'maintenance'
      | 'restart-required'
      | 'recovery-required'
      | 'restoration-failed';
  }
>;

function recoveryCommands(root: string): readonly string[] {
  return [
    `foks-rs --state-dir ${root} state status`,
    `foks-rs --state-dir ${root} state recover`,
  ];
}

function agentStopDetail(lifecycle: AgentStop): string {
  switch (lifecycle.state) {
    case 'maintenance':
      return 'FOKS is paused while background maintenance completes.';
    case 'recovery-required':
      return `The data directory at ${lifecycle.root} requires recovery. Run the following commands before reopening FOKS:`;
    case 'restart-required':
      return `Please restart FOKS to open ${lifecycle.root}.`;
    case 'restoration-failed':
      return lifecycle.error.message;
  }
}

/** The lifecycle as a stop state, or null while the agent runs or starts. */
function agentStop(lifecycle: AgentLifecycle): AgentStop | null {
  switch (lifecycle.state) {
    case 'maintenance':
    case 'restart-required':
    case 'recovery-required':
    case 'restoration-failed':
      return lifecycle;
    default:
      return null;
  }
}

type ShellBlock =
  | { kind: 'starting' }
  | { kind: 'locked' }
  | { kind: 'boot-error'; message: string }
  | { kind: 'stop'; lifecycle: AgentStop }
  | { kind: 'disconnected'; message?: string };

function shellBlock(
  lifecycle: AgentLifecycle,
  startup?: { locked: boolean; error: string | null; pending: boolean },
): ShellBlock | null {
  if (startup) {
    if (startup.locked) return { kind: 'locked' };
    if (startup.error !== null)
      return { kind: 'boot-error', message: startup.error };
    if (!startup.pending) return null;
    const stop = agentStop(lifecycle);
    return stop && stop.state !== 'maintenance'
      ? { kind: 'stop', lifecycle: stop }
      : { kind: 'starting' };
  }
  const stop = agentStop(lifecycle);
  if (stop) return { kind: 'stop', lifecycle: stop };
  return lifecycle.state === 'disconnected'
    ? { kind: 'disconnected', message: lifecycle.error }
    : null;
}

function shellChrome(
  block: ShellBlock | null,
  lifecycle: AgentLifecycle['state'] = 'ready',
  snapshotAgent?: AgentSnapshot['agent']['state'],
): { blocked: boolean; agent: RailAgentState } {
  return {
    blocked: block !== null,
    agent: !block
      ? railAgentState(lifecycle, snapshotAgent)
      : block.kind === 'locked'
        ? 'locked'
        : block.kind === 'starting' ||
            (block.kind === 'stop' && block.lifecycle.state === 'maintenance')
          ? 'starting'
          : 'stopped',
  };
}

function AgentStopCard({
  lifecycle,
  bridge,
  onRetryRestoration,
}: {
  lifecycle: AgentStop;
  bridge: Bridge;
  onRetryRestoration: () => void;
}): ReactNode {
  const [failure, setFailure] = useState<string | null>(null);
  const report = (error: unknown): void =>
    setFailure(normalizeCommandError(error).message);
  const label = agentLifecycleLabel(lifecycle);
  const quit = (
    <Button
      onClick={() => {
        void bridge.quitApp().catch(report);
      }}
    >
      Quit FOKS
    </Button>
  );
  const actions =
    lifecycle.state === 'restart-required' ? (
      // A blocking restart state provides both restart and quit actions.
      <>
        <Button
          variant="primary"
          onClick={() => {
            void bridge.restartApp().catch(report);
          }}
        >
          Restart FOKS
        </Button>
        {quit}
      </>
    ) : lifecycle.state === 'recovery-required' ? (
      quit
    ) : lifecycle.state === 'restoration-failed' ? (
      <>
        <Button variant="primary" onClick={onRetryRestoration}>
          Restart service
        </Button>
        {quit}
      </>
    ) : null;
  const body = (
    <>
      {lifecycle.state !== 'maintenance' ? (
        <p>{maintenanceOutcomeMessage(lifecycle.operation)}</p>
      ) : null}
      <p>{agentStopDetail(lifecycle)}</p>
      {lifecycle.state === 'recovery-required'
        ? recoveryCommands(lifecycle.root).map((command) => (
            <CopyBox
              key={command}
              text={command}
              onCopy={(text) => {
                void bridge.copyText(text).catch(report);
              }}
            >
              <code>{command}</code>
            </CopyBox>
          ))
        : null}
      {lifecycle.state === 'restart-required' ? (
        <p className="calm">
          Your vaults remain on this device and on their configured servers.
        </p>
      ) : null}
      {failure ? (
        <p className="action-error" role="alert">
          {failure}
        </p>
      ) : null}
      {actions ? <div className="acts2">{actions}</div> : null}
    </>
  );
  return (
    <div className="card stopcard">
      <h2>
        <Icon name="alert" />
        {label}
      </h2>
      {body}
    </div>
  );
}

function AgentLostCard({
  message,
  bridge,
  onRetryAgent,
}: {
  message?: string;
  bridge: Bridge;
  onRetryAgent: () => Promise<void>;
}): ReactNode {
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  return (
    <div className="notice stop">
      <h2>Connection to background service lost</h2>
      <p>
        The background service stopped responding. Click Retry to reconnect.
      </p>
      {message ? <p className="fn">{message}</p> : null}
      {failure ? (
        <p className="fn" role="alert">
          {failure}
        </p>
      ) : null}
      <div className="acts2">
        <Button
          variant="primary"
          disabled={busy}
          onClick={() => {
            setBusy(true);
            setFailure(null);
            void (async () => {
              try {
                await onRetryAgent();
              } catch (error) {
                setFailure(normalizeCommandError(error).message);
              } finally {
                setBusy(false);
              }
            })();
          }}
        >
          Retry
        </Button>
        <Button
          disabled={busy}
          onClick={() => {
            void bridge
              .quitApp()
              .catch((error) =>
                setFailure(normalizeCommandError(error).message),
              );
          }}
        >
          Quit FOKS
        </Button>
      </div>
    </div>
  );
}

function Takeover({
  block,
  children,
}: {
  block: ShellBlock;
  children?: ReactNode;
}): ReactNode {
  const modal = useHasOverlayProvider();
  const identity =
    block.kind === 'stop'
      ? `${block.kind}:${block.lifecycle.state}:${block.lifecycle.generation}`
      : block.kind;
  if (block.kind === 'starting')
    return (
      <div className="takeover">
        <StartingScreen />
      </div>
    );
  const className = `takeover ${
    block.kind === 'stop'
      ? 'stopveil'
      : block.kind === 'disconnected'
        ? 'stopwrap'
        : 'lock-back'
  }`;
  const label =
    block.kind === 'stop'
      ? agentLifecycleLabel(block.lifecycle)
      : block.kind === 'disconnected'
        ? 'Connection to background service lost'
        : block.kind === 'locked'
          ? 'FOKS is locked'
          : 'Couldn’t load FOKS';
  const role = block.kind === 'locked' ? 'dialog' : 'alertdialog';
  if (modal)
    return (
      <Dialog
        key={identity}
        className={className}
        role={role}
        aria-label={label}
      >
        {children}
      </Dialog>
    );
  // Before the shell mounts there is no overlay environment to isolate the
  // background from, and the frame behind this card is already inert.
  return (
    <div className={className} role={role} aria-modal="true" aria-label={label}>
      {children}
    </div>
  );
}

/**
 * Empty shell layout rendering a dimmed, disabled rail and topbar beside the
 * takeover container. Shared by starting, stopped, and locked states to
 * preserve window geometry across transitions.
 */
function BlockedShell({
  bridge,
  block,
  children,
}: {
  bridge?: Bridge | null;
  block: ShellBlock;
  children?: ReactNode;
}): ReactNode {
  const nowhere = (): void => undefined;
  const chrome = shellChrome(block);
  return (
    <div
      className={[
        'window',
        bridge?.native ? 'native-window' : 'web-mock-window',
      ].join(' ')}
    >
      <div className="app">
        <Sidebar
          location={{ kind: 'files' }}
          onNavigate={nowhere}
          agent={chrome.agent}
          nativeChrome={Boolean(bridge?.native)}
          blocked={chrome.blocked}
        />
        <main className="main">
          <Topbar
            location={{ kind: 'files' }}
            onNavigate={nowhere}
            collapsed={false}
            blocked={chrome.blocked}
          />
        </main>
      </div>
      <Takeover block={block}>{children}</Takeover>
    </div>
  );
}

/**
 * Mounts the search palette inside the chat inbox provider to share cached
 * channel names and keep search results consistent with rail unread badges.
 */
function ShellSearch({
  snapshot,
  open,
  onClose,
  onNavigate,
  onOpenItem,
}: {
  snapshot: AgentSnapshot;
  open: boolean;
  onClose: () => void;
  onNavigate: (location: Location) => void;
  onOpenItem: (store: string, path: string) => void;
}): ReactNode {
  const inbox = useSidebarInbox();
  const channels = useMemo(() => {
    const found: SearchChannel[] = [];
    for (const [store, team] of inbox)
      for (const conversation of team.data?.conversations ?? []) {
        if (conversation.hidden) continue;
        found.push({
          store,
          name: conversation.channel.name,
          id: conversation.channel.id,
        });
      }
    return found;
  }, [inbox]);
  return (
    <SearchPalette
      snapshot={snapshot}
      open={open}
      onClose={onClose}
      onNavigate={onNavigate}
      onOpenItem={onOpenItem}
      channels={channels}
    />
  );
}

/** The content area while the agent starts. */
function StartingScreen(): ReactNode {
  return (
    <div className="booting" role="status">
      <span className="spin" aria-hidden="true" />
      <b>Starting the FOKS agent…</b>
      <span className="line">Startup usually takes a few seconds.</span>
    </div>
  );
}

export interface AppProps {
  /** Supplying a snapshot makes render tests synchronous. Production omits it. */
  snapshot?: AgentSnapshot;
  /** A command seam for tests; production selects the Tauri or mock bridge. */
  bridge?: Bridge;
  /** Injected by the render tests so navigation is observable. */
  store?: LocationStore;
  /** Controlled wall clock for expiry lifecycle tests. */
  leaseClock?: LeaseExpiryClock;
}

export function App({
  snapshot: agentSnapshot,
  bridge,
  store,
  leaseClock,
}: AppProps): ReactNode {
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

  useEffect(() => {
    if (agentSnapshot) {
      setLoaded(agentSnapshot);
      if (bridge) {
        const controller = new AgentLifecycleController(
          bridge,
          agentSnapshot.agent,
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
    let handedOff = false;
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
        const current = (): boolean =>
          alive && !bootInvalidated && generation === bootGeneration.current;
        const publishPartial = (partial: AgentSnapshot): void => {
          if (
            !current() ||
            controller.snapshot().state !== 'ready' ||
            !partial.stores.length
          )
            return;
          stopMaintenance?.();
          stopMaintenance = undefined;
          handedOff = true;
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
            handedOff ||
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
            if (handedOff) publishPartial(next);
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
          !handedOff &&
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
        stopMaintenance?.();
        stopMaintenance = undefined;
        publishedBootGeneration.current = generation;
        setLoaded(next);
        setLoadError(null);
      } catch (error) {
        if (!alive || bootInvalidated || generation !== bootGeneration.current)
          return;
        if (handedOff && !isAgentReadinessError(normalizeCommandError(error)))
          return;
        setAgentLifecycle({ state: 'failure', error });
        setLoadError(normalizeCommandError(error).message);
      }
    })();
    return () => {
      alive = false;
      stopLifecycle?.();
      stopMaintenance?.();
    };
  }, [bootEpoch, bridge, agentSnapshot]);

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

  const block = shellBlock(agentLifecycle, {
    locked: Boolean(lockState && activeBridge),
    error: loadError,
    pending: !loaded || !activeBridge || !agentController,
  });

  if (block?.kind === 'locked' && lockState && activeBridge) {
    const mechanism =
      lockState.mechanism === 'biometry'
        ? 'Touch ID or your Mac password'
        : 'your operating-system password';
    return (
      <BlockedShell bridge={activeBridge} block={block}>
        <div className="card lockcard">
          <span className="glyph" aria-hidden="true">
            <Icon name="shield" />
          </span>
          <h2 id="app-lock-title">FOKS is locked</h2>
          <p>Authenticate with {mechanism} to unlock FOKS.</p>
          {lockError ? (
            <p className="action-error" role="alert">
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
      </BlockedShell>
    );
  }

  if (block?.kind === 'boot-error') {
    return (
      <BlockedShell bridge={activeBridge} block={block}>
        <div className="card lockcard">
          <span className="glyph warn" aria-hidden="true">
            <Icon name="alert" />
          </span>
          <h2 id="app-boot-error-title">Couldn’t load FOKS</h2>
          <p className="action-error" role="alert">
            {block.message}
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
      </BlockedShell>
    );
  }
  if (block?.kind === 'stop' && activeBridge) {
    return (
      <BlockedShell bridge={activeBridge} block={block}>
        <AgentStopCard
          lifecycle={block.lifecycle}
          bridge={activeBridge}
          onRetryRestoration={() => {
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
                if (current.state === 'checking' || current.state === 'ready')
                  return;
                setLoadError(normalizeCommandError(error).message);
              });
          }}
        />
      </BlockedShell>
    );
  }
  if (
    block?.kind === 'starting' ||
    !loaded ||
    !activeBridge ||
    !agentController
  )
    return <BlockedShell bridge={activeBridge} block={{ kind: 'starting' }} />;
  return (
    <VaultShell
      snapshot={loaded}
      bridge={activeBridge}
      store={store}
      firstRunStart={firstRunStart}
      managedProfile={managedProfile}
      onLock={lockNow}
      retireBoot={retireBoot}
      currentBootSnapshot={currentBootSnapshot}
      agentController={agentController}
      leaseClock={leaseClock}
    />
  );
}

/** A `prompt` verdict on screen, with the promise the store is waiting on. */
interface PendingPrompt {
  verdict: NavigationPromptVerdict;
  settle: (confirmed: boolean) => void;
}

interface VaultShellProps {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  store?: LocationStore;
  firstRunStart?: 'who' | 'local' | null;
  managedProfile?: string | null;
  onLock: () => Promise<boolean>;
  retireBoot: () => void;
  currentBootSnapshot: () => boolean;
  agentController: AgentLifecycleController;
  leaseClock?: LeaseExpiryClock;
}

function VaultShell({
  snapshot: agentSnapshot,
  bridge,
  store,
  firstRunStart = null,
  managedProfile = null,
  onLock,
  retireBoot,
  currentBootSnapshot,
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
    () => scene.selection ?? demoSelection(scene.demo, agentSnapshot),
  );
  const [deviceLabel, setDeviceLabel] = useState<DeviceLabel | null>(null);
  const [sideCollapsed, setSideCollapsed] = useState(storedSideCollapsedPref);
  // Initialize the rail color theme from local storage on initial render.
  // Settings applies preference changes directly upon user selection.
  useEffect(() => {
    applyRailColor(storedRailColor());
  }, []);
  // The topbar's toggle is the only writer of the stored preference; the
  // details panel's reaction below changes the width without recording it.
  const toggleSidebar = useCallback(() => {
    const collapsed = !sideCollapsed;
    setSideCollapsed(collapsed);
    rememberSideCollapsed(collapsed);
  }, [sideCollapsed]);
  const [searchOpen, setSearchOpen] = useState(false);
  useSearchShortcut(() => setSearchOpen(true));
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
  const [latest, setLatest] = useState(agentSnapshot);
  const [, updateSetupProgress] = useState(0);
  useEffect(() => {
    const update = () => updateSetupProgress((value) => value + 1);
    window.addEventListener(FIRST_RUN_PROGRESS_EVENT, update);
    return () => window.removeEventListener(FIRST_RUN_PROGRESS_EVENT, update);
  }, []);
  const latestRef = useRef(latest);
  latestRef.current = latest;
  const [observedExpiredLeases, setObservedExpiredLeases] = useState(
    agentSnapshot.observedExpiredLeases,
  );
  const [agentLifecycle, setAgentLifecycle] = useState<AgentLifecycle>(() =>
    agentController.snapshot(),
  );
  const [agentCatalogReady, setAgentCatalogReady] = useState(true);
  const [refreshingSnapshot, setRefreshingSnapshot] = useState(false);
  const [workflow, setWorkflow] = useTabSheetState<WriteWorkflow>(
    'write.workflow',
    () =>
      initialWriteWorkflow(
        typeof window === 'undefined' ? '' : window.location.search,
        agentSnapshot,
      ),
    (value) => value?.kind === 'new',
    locations,
  );
  const [toasts] = useState(() => new ToastController());
  // Handles navigation guard outcomes: `prompt` renders the confirmation dialog
  // below, and `refuse` displays an alert toast. The active pending navigation is
  // stored in a ref as well as in state so a newer intent can cancel it without
  // a stale closure.
  const [prompt, setPrompt] = useState<PendingPrompt | null>(null);
  const promptRef = useRef<PendingPrompt | null>(null);
  const settlePrompt = useCallback((confirmed: boolean) => {
    const open = promptRef.current;
    if (!open) return;
    promptRef.current = null;
    setPrompt(null);
    open.settle(confirmed);
  }, []);
  useEffect(() => {
    locations.setPrompter(
      (verdict) =>
        new Promise<boolean>((resolve) => {
          const open = promptRef.current;
          const next = { verdict, settle: resolve };
          promptRef.current = next;
          setPrompt(next);
          // Replace any existing confirmation prompt and resolve its promise
          // as cancelled.
          open?.settle(false);
        }),
    );
    locations.setRefusalHandler((reason) => {
      toasts.show(reason, { tone: 'warning' });
    });
    return () => {
      locations.setPrompter(null);
      locations.setRefusalHandler(null);
      // Resolve any pending navigation prompt as cancelled on unmount.
      settlePrompt(false);
    };
  }, [locations, settlePrompt, toasts]);
  const [concealSignal, setConcealSignal] = useState(0);
  const metadataInvalidation = useRef<() => void>(() => undefined);
  const [hardwareRefresh, setHardwareRefresh] = useState(0);
  // A conceal ends the session the remembered chat belonged to: the account
  // that comes back may not have that team on this Mac, so the rail's Chat tab
  // runs the first-team fallback again instead of reopening it. The Chat tab
  // is keyed by the same signal, so its own memory is written after this.
  useEffect(() => {
    if (!concealSignal) return;
    rememberChatLocation(null);
    locations.clearTabMemory();
  }, [concealSignal, locations]);
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
  // Scenes trigger only on initial navigation. Conceal events remount tabs to
  // clear sensitive inputs, so tabs must not re-trigger scene dialogs.
  const enteredScene = concealSignal === 0 ? namedState : '';

  const fixtureFirstRunBoot = Boolean(
    bridge.firstRunFixture &&
    state.location.kind === 'first-run' &&
    state.location.step === 'boot',
  );
  // Apply URL query lease overrides to the active snapshot. Ordinary
  // navigation must not manufacture a new snapshot: first-run treats a changed
  // snapshot as authoritative inventory and could otherwise rewind a server
  // profile that the preceding command just created.
  const shown = useMemo(() => {
    const reconciled = { ...latest, observedExpiredLeases };
    const leased =
      scene.lease === 'lapsed' ? applyLease(reconciled, 'lapsed') : reconciled;
    const demonstrated = demoAvailabilityFacts(leased, namedState);
    if (fixtureFirstRunBoot) {
      return {
        ...demonstrated,
        agent: { state: 'bootstrap' as const, step: 'create-state' },
      };
    }
    return demonstrated;
  }, [
    fixtureFirstRunBoot,
    latest,
    namedState,
    observedExpiredLeases,
    scene.lease,
  ]);

  useEffect(
    () => locations.setAccountStores(shown.stores),
    [locations, shown.stores],
  );
  useEffect(() => {
    if (currentBootSnapshot()) setLatest(agentSnapshot);
  }, [agentSnapshot, currentBootSnapshot]);

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

  const catalogCoordinator = useMemo(
    () =>
      new CatalogCoordinator<AgentSnapshot>(
        (onPartial, isCurrent) =>
          loadSnapshot(
            bridge,
            latestRef.current,
            undefined,
            onPartial,
            isCurrent,
          ),
        (next, forced) => {
          latestRef.current = next;
          setLatest(next);
          if (forced) {
            metadataInvalidation.current();
            setHardwareRefresh((generation) => generation + 1);
          }
          setAgentCatalogReady(true);
        },
      ),
    [bridge],
  );
  useEffect(() => {
    catalogCoordinator.activate();
    return () => catalogCoordinator.deactivate();
  }, [catalogCoordinator]);
  const refreshSnapshot = useCallback(
    (force = false): Promise<AgentSnapshot> => {
      retireBoot();
      return catalogCoordinator.refresh(force);
    },
    [catalogCoordinator, retireBoot],
  );

  const refresh = useCallback(
    async (message: string): Promise<void> => {
      const result = await synchronizeApplied(() => refreshSnapshot(true));
      if (result.synchronization === 'pending') {
        if (result.error.code === 'catalog-read-retired') return;
        if (isAgentReadinessError(result.error))
          commandErrorRef.current(result.error);
        else
          toasts.show(
            'Change completed. Updated data could not be loaded. Use Refresh to reload it.',
            { tone: 'warning' },
          );
        return;
      }
      toasts.show(message);
    },
    [refreshSnapshot, toasts],
  );

  const refreshSnapshotRef = useRef(refreshSnapshot);
  const commandErrorRef = useRef<(error: unknown) => void>(() => undefined);
  const foregroundRefreshAllowed = useRef(false);
  refreshSnapshotRef.current = refreshSnapshot;
  foregroundRefreshAllowed.current =
    latest.agent.state === 'ready' &&
    agentController.snapshot().state === 'ready' &&
    agentCatalogReady;

  const disconnectAgent = useCallback(
    (message: string): void => {
      if (!agentController.disconnect(message)) return;
      foregroundRefreshAllowed.current = false;
      retireBoot();
      catalogCoordinator.reset();
      setAgentCatalogReady(false);
      setConcealSignal((value) => value + 1);
    },
    [agentController, catalogCoordinator, retireBoot],
  );

  const recoverAgentReadiness = useCallback(
    async (reconnect: boolean): Promise<void> => {
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
    [agentController, refreshSnapshot],
  );

  const commandError = useCallback(
    (error: unknown, item?: Item, draft = ''): void => {
      const typed = normalizeCommandError(error);
      if (typed.code === 'catalog-read-retired') return;
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
    [disconnectAgent, toasts, setWorkflow],
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
          void refreshSnapshotRef
            .current(true)
            .catch((error: unknown) => commandErrorRef.current(error));
      },
    );
    expiryCoordinator.current = coordinator;
    const reconcileForeground = (): void => {
      coordinator.foreground();
      if (foregroundRefreshAllowed.current)
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
        async () => {
          await refreshSnapshot(true);
        },
        commandError,
      );
    },
    [commandError, refreshSnapshot],
  );

  /**
   * Saves a file dropped on a vault's content area. Group items take the same
   * default roles the new-item sheet offers; a path already in use reopens
   * that sheet's conflict step with the dropped file still chosen.
   */
  const uploadDroppedFile = useCallback(
    async ({ storeId, path, sourcePath }: DropUpload): Promise<void> => {
      const target = storeOf(shown, storeId);
      try {
        await bridge.importDroppedFile({
          storeId,
          path,
          sourcePath,
          ...(target?.kind === 'team'
            ? { readRole: DEFAULT_READ_ROLE, writeRole: DEFAULT_WRITE_ROLE }
            : {}),
        });
        await refresh(
          `Uploaded ${nameOf(path)}${target ? ` to ${target.name}` : ''}`,
        );
      } catch (error) {
        if (normalizeCommandError(error).code === 'already-exists') {
          setWorkflow({
            kind: 'exists',
            itemKind: 'Document',
            storeId,
            path,
            draft: droppedFileDraft(path, sourcePath),
          });
          await mutationError(error, { report: false });
        } else await mutationError(error);
      }
    },
    [bridge, mutationError, refresh, shown, setWorkflow],
  );

  const refreshAll = (): void => {
    if (refreshingSnapshot) return;
    setRefreshingSnapshot(true);
    void refreshSnapshot(true)
      .then(() => toasts.show('Vaults and teams refreshed'))
      .catch(commandError)
      .finally(() => setRefreshingSnapshot(false));
  };

  useEffect(() => {
    return agentController.subscribe(setAgentLifecycle);
  }, [agentController]);

  useEffect(() => {
    if (!bridge.native) return;
    let alive = true;
    let stop: (() => void) | undefined;
    const apply = (
      snapshot: Awaited<ReturnType<Bridge['clientStateMaintenanceStatus']>>,
    ): void => {
      if (!alive) return;
      if (!agentController.applyMaintenance(snapshot)) return;
      if (snapshot.state === 'idle') return;
      foregroundRefreshAllowed.current = false;
      retireBoot();
      catalogCoordinator.reset();
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
  }, [
    agentController,
    bridge,
    catalogCoordinator,
    commandError,
    refreshSnapshot,
    retireBoot,
    toasts,
  ]);

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
      if (error.code === 'agent-lost') {
        disconnectAgent(error.message);
        return;
      }
      foregroundRefreshAllowed.current = false;
      retireBoot();
      catalogCoordinator.reset();
      setAgentCatalogReady(false);
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
    [
      agentController,
      catalogCoordinator,
      commandError,
      disconnectAgent,
      recoverAgentReadiness,
      retireBoot,
      state.location.kind,
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
        const message = await bridge.takeAgentConnectionLoss();
        if (alive && message) disconnectAgent(message);
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

  const appRef = useRef<HTMLDivElement>(null);
  const portalRoot = useMemo(
    () =>
      typeof document === 'undefined'
        ? null
        : (document.getElementById('overlays') ?? document.body),
    [],
  );
  const lockFromMenu = () => {
    void onLock().then(
      (locked) => {
        if (!locked)
          toasts.show('Application lock is not available on this system.');
      },
      (error: unknown) => commandError(error),
    );
  };
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
  // The checklist screens draw the shell's own rail, collapse toggle and all.
  // The setup steps draw a rail with no toggle, which stays open.
  const firstRunRailCollapsible =
    here.kind === 'first-run' &&
    ['added', 'checklist-invited', 'checklist-own'].includes(here.step ?? '');
  const railCollapsed =
    sideCollapsed && (here.kind !== 'first-run' || firstRunRailCollapsible);
  const trafficLightsVisible = !railCollapsed;
  useEffect(() => {
    if (bridge.native)
      void bridge
        .setTrafficLightsVisible(trafficLightsVisible)
        .catch(commandError);
  }, [bridge, commandError, trafficLightsVisible]);
  // Handle trackpad back gestures consistently with the topbar back button.
  // The destination is read through a ref so the listener mounts once. Suppress
  // gestures while a dialog is open or a navigation guard blocks the target to
  // avoid opening confirmation dialogs or displaying errors from gestures.
  const hereRef = useRef(here);
  hereRef.current = here;
  useEffect(
    () =>
      mountSwipeBack({
        target: () => parentLocation(hereRef.current),
        enabled: () => {
          if (anyDialogOpen()) return false;
          const parent = parentLocation(hereRef.current);
          return (
            parent !== null &&
            locations.navigationVerdict({
              kind: 'navigate',
              location: parent,
            }) === null
          );
        },
        navigate: (location) => locations.navigate(location),
      }),
    [locations],
  );
  // A prompt asks whether to leave the page it was raised on. If the shell
  // left it some other way, the question no longer applies.
  useEffect(() => {
    settlePrompt(false);
  }, [here, settlePrompt]);

  const pendingFirstRun = incompleteFirstRunCheckpoint();
  const detailsShown =
    state.details &&
    listsItems(here) &&
    !(here.kind === 'store' && !storeReadable(shown, here.ref)) &&
    !(state.selection && !storeReadable(shown, state.selection.store));
  // Adjust rail width when details visibility changes. Opening details
  // collapses the rail; closing details restores it. This transient layout
  // state is not persisted, and an initially open details panel collapses the
  // rail once.
  const detailsWasShown = useRef(false);
  useEffect(() => {
    if (detailsWasShown.current === detailsShown) return;
    detailsWasShown.current = detailsShown;
    setSideCollapsed(detailsShown);
  }, [detailsShown]);
  // Scrollbars show while a pane is scrolling and for 700ms after. Scroll
  // events do not bubble, so the listener is capture-phase; that also covers
  // keyboard scrolling, which no pointer event would report.
  useEffect(() => {
    const timers = new WeakMap<Element, number>();
    const onScroll = (event: Event): void => {
      const pane = event.target;
      if (!(pane instanceof HTMLElement)) return;
      pane.classList.add('scrolling');
      window.clearTimeout(timers.get(pane));
      timers.set(
        pane,
        window.setTimeout(() => pane.classList.remove('scrolling'), 700),
      );
    };
    document.addEventListener('scroll', onScroll, true);
    return () => document.removeEventListener('scroll', onScroll, true);
  }, []);
  const selectedAccessGeneration = state.selection
    ? (accessGenerations.get(
        storeOf(shown, state.selection.store)?.server ?? '',
      ) ?? 0)
    : 0;
  // The rail's own badges: Teams' and Devices' each read a registry the page
  // that already loads the underlying fact reports into, rather than asking
  // the agent again from here. See `team-requests.ts` and `device-alert.ts`.
  const teamRequestCounts = useSyncExternalStore(
    teamRequestRegistry(bridge).subscribe,
    teamRequestRegistry(bridge).getSnapshot,
  );
  const deviceAlerts = useSyncExternalStore(
    deviceAlertRegistry(bridge).subscribe,
    deviceAlertRegistry(bridge).getSnapshot,
  );
  // People, Teams, Devices and Settings are the same body under four titles.
  const settingsProps = {
    snapshot: shown,
    bridge,
    scene: enteredScene,
    onNavigate: (location: Location, options?: NavigateOptions) =>
      locations.navigate(location, options),
    onRefresh: refresh,
    onRefreshSnapshot: refreshSnapshot,
    onError: commandError,
    onMutationError: mutationError,
    onLock,
    agentLifecycle,
    onRetryAgent: () => recoverAgentReadiness(true),
  };
  const screen = listsItems(here) ? (
    <ItemsScreen
      snapshot={shown}
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
          await refresh('Team creation resumed');
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
      onUploadDroppedFile={uploadDroppedFile}
      // A modal workflow owns the drop while it is open; the new-item sheet
      // takes dropped files itself.
      dropEnabled={workflow === null}
      accessNow={accessNow}
    />
  ) : here.kind === 'chat' ? (
    // The tab owns the team column and keeps it across a switch; the
    // conversation beside it is what remounts with the team.
    <ChatTab
      key={`chat:${concealSignal}`}
      snapshot={shown}
      bridge={bridge}
      location={here}
      accessNow={accessNow}
      // The generation belongs to the server of the team the tab opens, and
      // with no `ref` the tab is the only thing that knows which team that is,
      // so it is handed the whole map and picks from the `ref` it resolved.
      accessGenerations={accessGenerations}
      onNavigate={(location, options) => locations.navigate(location, options)}
    />
  ) : here.kind === 'group-settings' ? (
    <GroupSettingsScreen
      key={`${here.kind}:${here.ref}`}
      snapshot={shown}
      bridge={bridge}
      location={here}
      // The Channels tab reads chat, so it decides availability on the shell's
      // own clock and guards a new channel on the same access generation the
      // Chat tab does.
      accessNow={accessNow}
      accessGenerations={accessGenerations}
      onNavigate={(location, options) => locations.navigate(location, options)}
      onApplied={refresh}
      onError={commandError}
      onMutationError={mutationError}
    />
  ) : here.kind === 'people' ? (
    // People owns its own sheets and shares no state with Settings, so it is
    // given exactly the props it declares.
    <PeopleScreen
      onLock={lockFromMenu}
      key={`people:${concealSignal}`}
      snapshot={shown}
      bridge={bridge}
      location={here}
      onNavigate={(location, options) => locations.navigate(location, options)}
      onRefresh={refresh}
      onRefreshSnapshot={refreshSnapshot}
      onError={commandError}
      onMutationError={mutationError}
    />
  ) : here.kind === 'teams' ? (
    // Teams shares no state with Settings, so it is given exactly the props it
    // declares rather than the settings bundle.
    <TeamsScreen
      key={`teams:${concealSignal}`}
      snapshot={shown}
      bridge={bridge}
      location={here}
      scene={namedState}
      onNavigate={(location) => locations.navigate(location)}
      onRefresh={refresh}
      onRefreshSnapshot={refreshSnapshot}
      onError={commandError}
      onMutationError={mutationError}
    />
  ) : here.kind === 'devices' ? (
    <DevicesScreen
      hardwareRefresh={hardwareRefresh}
      onDeviceLabel={setDeviceLabel}
      key={`devices:${concealSignal}`}
      snapshot={shown}
      bridge={bridge}
      location={here}
      scene={enteredScene}
      onNavigate={(location, options) => locations.navigate(location, options)}
      onRefresh={refresh}
      onRefreshSnapshot={refreshSnapshot}
      onError={commandError}
      onMutationError={mutationError}
    />
  ) : here.kind === 'settings' ? (
    <SettingsScreen
      key={`settings:${concealSignal}`}
      {...settingsProps}
      location={here}
    />
  ) : (
    <PlaceholderScreen location={here} />
  );

  // Render takeover overlays as siblings of the main grid so they remain
  // interactive while the grid is inert. The adjacent rail and topbar use the
  // same disabled styling as `BlockedShell`.
  const block = shellBlock(agentLifecycle);
  const chrome = shellChrome(block, agentLifecycle.state, shown.agent.state);
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
      <div
        className={[
          'app',
          detailsShown || (here.kind === 'first-run' && here.step === 'added')
            ? 'with-details'
            : '',
          // The setup steps replace the rail with one that has no toggle; the
          // checklist screens draw the rail itself and collapse with it.
          railCollapsed ? 'side-narrow' : '',
        ]
          .filter(Boolean)
          .join(' ')}
        ref={appRef}
      >
        <ChatInboxProvider
          key={`inbox:${concealSignal}`}
          bridge={bridge}
          snapshot={shown}
          onNavigate={navigateFromNotification}
          clock={chatClock}
          accessNow={accessNow}
        >
          {here.kind === 'first-run' ? (
            <FirstRunExperience
              snapshot={shown}
              bridge={bridge}
              location={here}
              onNavigate={(location) => locations.navigate(location)}
              onRefreshSnapshot={refreshSnapshot}
              concealSignal={concealSignal}
              agentReady={
                shown.agent.state === 'ready' &&
                agentLifecycle.state === 'ready' &&
                agentCatalogReady
              }
              onRetryAgent={() =>
                recoverAgentReadiness(agentLifecycle.state === 'disconnected')
              }
              onAgentReadinessFailure={handleAgentReadinessFailure}
              automaticEntry={automaticFirstRun}
              managedProfile={managedProfile ?? undefined}
              agent={chrome.agent}
              blocked={chrome.blocked}
              collapsed={sideCollapsed}
              onToggleCollapsed={toggleSidebar}
              devicesAlert={devicesAlertSummary(shown, deviceAlerts)}
            />
          ) : (
            <>
              <Sidebar
                snapshot={shown}
                location={here}
                folder={state.folder}
                account={locations.getAccount()}
                attention={unroutedNotices(shown).length}
                teamRequests={teamRequestsBadge(shown, teamRequestCounts)}
                devicesAlert={devicesAlertSummary(shown, deviceAlerts)}
                settingsAlert={settingsAlertSummary(shown)}
                onTabNavigate={(tab) => locations.navigateTab(tab)}
                onNavigate={(location) => {
                  locations.navigate(location);
                }}
                onSetFolder={(folder) => locations.setFolder(folder)}
                onLock={lockFromMenu}
                status={
                  pendingFirstRun ? (
                    <FirstRunChecklistStatus
                      checkpoint={pendingFirstRun}
                      onNavigate={(location) => locations.navigate(location)}
                    />
                  ) : undefined
                }
                onToggleCollapsed={toggleSidebar}
                collapsed={sideCollapsed}
                // The lifecycle is the rail's own authority for whether the
                // agent is gone, whichever path raised the disconnect.
                agent={chrome.agent}
                nativeChrome={bridge.native}
                blocked={chrome.blocked}
              />
              <main className="main">
                <Topbar
                  blocked={chrome.blocked}
                  deviceLabel={deviceLabel}
                  snapshot={shown}
                  location={here}
                  folder={state.folder}
                  onNavigate={(location) => locations.navigate(location)}
                  onSetFolder={(folder) => locations.setFolder(folder)}
                  onSearch={() => setSearchOpen(true)}
                  collapsed={sideCollapsed}
                  refreshing={refreshingSnapshot}
                  onRefresh={refreshAll}
                />
                {screen}
              </main>
            </>
          )}
          {here.kind !== 'first-run' && detailsShown ? (
            <DetailsPanel
              snapshot={shown}
              bridge={bridge}
              revealRequest={revealRequest}
              onRevealHandled={() => setRevealRequest(null)}
              selection={state.selection}
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
          <ShellSearch
            snapshot={shown}
            open={searchOpen}
            onClose={() => setSearchOpen(false)}
            onNavigate={(location) => locations.navigate(location)}
            onOpenItem={(storeId, path) =>
              locations.navigateAndSelect(
                { kind: 'store', ref: storeId },
                { store: storeId, path },
              )
            }
          />
        </ChatInboxProvider>
      </div>
      <WriteOverlay
        snapshot={shown}
        accessNow={accessNow}
        bridge={bridge}
        workflow={workflow}
        setWorkflow={setWorkflow}
        onApplied={refresh}
        onError={commandError}
        onMutationError={mutationError}
        onRefreshConflict={async (item, draft) => {
          await refreshSnapshot();
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
          const next = await refreshSnapshot();
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
          // The write workflow resolved this destination and is closing, so
          // underlying screens must not block this navigation.
          locations.select(
            { store: current.store, path: current.path },
            { force: true },
          );
        }}
      />
      {prompt ? (
        <NavigationPrompt
          verdict={prompt.verdict}
          onConfirm={() => settlePrompt(true)}
          onCancel={() => settlePrompt(false)}
        />
      ) : null}
      {block ? (
        <Takeover block={block}>
          {block.kind === 'stop' ? (
            <AgentStopCard
              lifecycle={block.lifecycle}
              bridge={bridge}
              onRetryRestoration={() => {
                void recoverAgentReadiness(true).catch(commandError);
              }}
            />
          ) : block.kind === 'disconnected' ? (
            <AgentLostCard
              message={block.message}
              bridge={bridge}
              onRetryAgent={() => recoverAgentReadiness(true)}
            />
          ) : null}
        </Takeover>
      ) : null}
    </div>
  );

  // Display labels do not identify accounts. Replacing identities or access
  // retires all cached metadata. Ordinary publication keeps resource data; a
  // forced full refresh invalidates it after the new catalog is installed.
  const deviceIdentity = JSON.stringify({
    accounts: shown.accounts.map(({ store, alias, server }) => [
      store,
      alias,
      server,
    ]),
    servers: shown.servers.map(({ id, host_id, configuredProbe }) => [
      id,
      host_id,
      configuredProbe,
    ]),
    access: shown.stores
      .filter((store) => store.kind === 'account')
      .map((store) => [store.id, accountStopped(shown, store).stopped]),
    generations: [...accessGenerations],
  });
  const deviceCache = useMemo(
    () => new DeviceCache(bridge),
    // These values define the lifetime of the cache, not its read arguments.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [bridge, concealSignal, deviceIdentity],
  );
  metadataInvalidation.current = () => deviceCache.repository.invalidate([]);
  deviceCache.repository.setReadRecovery(
    () =>
      readRecoveryFor(deviceCache.repository).options({
        refresh: () => refreshSnapshot(),
      }),
    () => foregroundRefreshAllowed.current,
  );
  useEffect(() => () => deviceCache.clear(), [deviceCache]);

  const withToasts = (
    <ToastProvider controller={toasts} portalRoot={portalRoot}>
      <NavigationGuardProvider store={locations}>
        <QueryRepositoryContext.Provider value={deviceCache.repository}>
          <DeviceCacheContext.Provider value={deviceCache}>
            {shell}
          </DeviceCacheContext.Provider>
        </QueryRepositoryContext.Provider>
      </NavigationGuardProvider>
    </ToastProvider>
  );
  if (!portalRoot) return withToasts;
  return (
    <OverlayProvider
      backgroundRef={appRef}
      portalRoot={portalRoot}
      blocking={block !== null}
    >
      {withToasts}
    </OverlayProvider>
  );
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

/** Applies review-scene failures once at the fixture/model boundary. */
function demoAvailabilityFacts(
  agentSnapshot: AgentSnapshot,
  state: string,
): AgentSnapshot {
  if (
    state === 'servers-list' ||
    state === 'servers-add' ||
    state === 'servers-lapsed'
  )
    return applyLease(agentSnapshot, 'lapsed');
  if (state === 'servers-rollback') {
    const error = {
      code: 'host-verification-failed',
      message: 'The fixture host identity moved backwards.',
      fatal: false,
      retryable: false,
      ambiguous: false,
    };
    return {
      ...agentSnapshot,
      servers: agentSnapshot.servers.map((server) =>
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
      ...agentSnapshot,
      storeInventory: agentSnapshot.storeInventory.map((entry) =>
        entry.store === 'acct:work'
          ? { ...entry, status: 'unavailable' as const, error }
          : entry,
      ),
    };
  }
  return agentSnapshot;
}

function demoSelection(
  demo: Scene['demo'],
  agentSnapshot: AgentSnapshot,
): { store: string; path: string } | null {
  if (!demo) return null;
  const candidates = agentSnapshot.items.filter(
    (item) => item.kind !== 'Folder',
  );
  let item: Item | undefined;
  if (demo === 'password')
    item = candidates.find((candidate) => isLogin(candidate));
  else if (demo === 'resource') {
    item = candidates
      .filter(
        (candidate) =>
          candidate.kind === 'Secret' &&
          kindOf(candidate) === 'Document' &&
          storeOf(agentSnapshot, candidate.store)?.kind === 'account',
      )
      .sort((left, right) =>
        nameOf(left.path).localeCompare(nameOf(right.path)),
      )[0];
  } else if (demo === 'file') {
    item = candidates
      .filter(
        (candidate) =>
          candidate.kind === 'File' &&
          storeOf(agentSnapshot, candidate.store)?.kind === 'team',
      )
      .sort((left, right) => right.version - left.version)[0];
  } else {
    item = candidates
      .filter(
        (candidate) =>
          kindOf(candidate) === 'Password' &&
          storeOf(agentSnapshot, candidate.store)?.kind === 'team',
      )
      .sort((left, right) =>
        nameOf(left.path).localeCompare(nameOf(right.path)),
      )[0];
  }
  return item ? { store: item.store, path: item.path } : null;
}
