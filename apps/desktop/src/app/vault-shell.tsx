import { TeamInfoPanel } from '../screens/team-info';
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from 'react';
import { anyDialogOpen, OverlayProvider } from '../../kit/overlay-primitives';
import { ToastController, ToastProvider } from '../../kit/toasts';
import type { AgentLifecycleController } from '../agent-lifecycle';
import { attentionItems, setupChecklist } from '../attention';
import type { AppLockState, Bridge, ItemField } from '../bridge';
import { useChatMigration } from '../chat/migration';
import { ChatInboxProvider } from '../chat/inbox-provider';
import { DeviceCacheContext } from '../device-cache';
import {
  FIRST_RUN_PROGRESS_EVENT,
  storedFirstRunCheckpoint,
} from '../first-run-operations';
import {
  completedFirstRunSteps,
  firstRunNextStep,
  firstRunStepCount,
  type FirstRunCheckpoint,
} from '../first-run-state';
import {
  storeAtScene,
  useLocationState,
  type KindFilter,
  type LocationStore,
} from '../location';
import {
  applyLease,
  defaultCreateStore,
  isTrashPath,
  settingsAlertSummary,
  storeOf,
  storeReadable,
  type AgentSnapshot,
  type DeviceLabel,
} from '../model';
import { NavigationGuardProvider, useTabSheetState } from '../navigation-guard';
import { MetadataRepositoryContext } from '../query-hooks';
import { followMoves } from '../recent-favorites';
import {
  systemLeaseExpiryClock,
  type LeaseExpiryClock,
} from '../scheduling/lease-expiry';
import {
  deviceAlertRegistry,
  devicesAlertSummary,
} from '../screens/device-alert';
import {
  recoveryPhraseTests,
  subscribeRecoveryPhraseTests,
} from '../recovery-phrase-tests';
import { DetailsPanel } from '../screens/details-panel';
import { FirstRunExperience } from '../screens/first-run-screen';
import { unroutedNotices } from '../screens/account-section';
import {
  ALL_ITEMS,
  folderKey,
  folderSelection,
  listsItems,
} from '../screens/scope';
import { copyAndAnnounce, openWebsiteAndAnnounce } from '../screens/item-copy';
import { exportedPasswords } from '../screens/export-passwords-sheet';
import { itemActionProblem } from '../screens/store-access';
import {
  teamRequestsBadge,
  usePendingMembershipCounts,
  useTeamRequestCounts,
} from '../operation-queries';
import {
  initialWriteWorkflow,
  writeBlockReason,
  type WriteWorkflow,
} from '../screens/write-workflows';
import { useSearchShortcut } from '../shell/search-palette';
import { ShortcutSheet, useShortcutSheetKey } from '../shell/shortcut-sheet';
import { matchesShortcut, shortcutLabel } from '../shortcuts';
import type { SearchAction } from '../shell/search-palette';
import { Sidebar } from '../shell/sidebar';
import { Topbar } from '../shell/topbar';
import { requestNewChat } from '../screens/chat-tab';
import { requestChatFind } from '../chat/find';
import { chatTeams } from '../screens/chat-teams';
import { useAccessRuntime } from './access-runtime';
import { AccessLifetime } from './access-lifetime';
import { shellBlock, shellChrome } from './blocking-shell';
import { useCatalogRuntime, useMutationError } from './catalog-runtime';
import type { MaintenanceOwnership } from './maintenance-ownership';
import { useMetadataRuntime } from './metadata-runtime';
import { DeviceMetadataContext } from '../device-metadata';
import { diagnosticLog } from '../diagnostics/log';
import {
  backendTimingSource,
  subscribeDiagnostics,
} from '../diagnostics/subscribe';
import { useShellNavigation } from './navigation-runtime';
import {
  demoAvailabilityFacts,
  demoSelection,
  initialScene,
  fixtureScenesAllowed,
} from './scenes';
import { PanelErrorBoundary } from './fault-boundaries';
import { ScreenRouter } from './screen-router';
import {
  ScreenErrorBoundary,
  screenIdentity,
  screenBoundaryKey,
} from './screen-error-boundary';
import {
  ShellOverlays,
  useDroppedUpload,
  type ResumeDraft,
} from './shell-overlays';
import { useTrashActions } from './trash-actions';
import { useShellRuntime } from './shell-runtime';
import { ShellSearch } from './shell-search';
import { useWindowRuntime } from './window-runtime';

function incompleteFirstRunCheckpoint(): FirstRunCheckpoint | null {
  const checkpoint = storedFirstRunCheckpoint();
  return checkpoint &&
    completedFirstRunSteps(checkpoint) < firstRunStepCount(checkpoint)
    ? checkpoint
    : null;
}

interface VaultShellProps {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  store?: LocationStore;
  firstRunStart?: 'who' | 'local' | null;
  managedProfile?: string | null;
  initialDeviceCache?: import('../device-cache').DeviceCache;
  onLock: () => Promise<boolean>;
  retireBoot: () => void;
  /** Resolves when the active boot catalog load completes, or immediately if none is running. */
  awaitBootRead?: () => Promise<void>;
  currentBootSnapshot: () => boolean;
  agentController: AgentLifecycleController;
  maintenanceOwnership: MaintenanceOwnership;
  leaseClock?: LeaseExpiryClock;
}

export function VaultShell({
  snapshot: agentSnapshot,
  bridge,
  store,
  firstRunStart = null,
  managedProfile = null,
  initialDeviceCache,
  onLock: requestLock,
  retireBoot,
  awaitBootRead,
  currentBootSnapshot,
  agentController,
  maintenanceOwnership,
  leaseClock = systemLeaseExpiryClock,
}: VaultShellProps): ReactNode {
  const lifetime = useMemo(() => new AccessLifetime(bridge), [bridge]);
  useSyncExternalStore(
    lifetime.subscribe,
    lifetime.getSnapshot,
    lifetime.getSnapshot,
  );
  useEffect(() => () => lifetime.retire('access-change'), [lifetime]);
  const [toasts] = useState(() => new ToastController());
  // The attention centre's Recent list is the toast history. A lock also
  // unmounts this shell, which discards the controller; clearing it here does
  // not depend on that.
  const onLock = useCallback(async () => {
    const locked = await requestLock();
    if (locked) {
      lifetime.retire('lock');
      toasts.clearHistory();
    }
    return locked;
  }, [lifetime, requestLock, toasts]);
  const fixtures = fixtureScenesAllowed(bridge);
  const [{ scene, automaticFirstRun }] = useState(() => {
    const decoded = initialScene(bridge);
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
  const [searchOpen, setSearchOpen] = useState(false);
  useSearchShortcut(() => setSearchOpen(true));
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  useShortcutSheetKey(() => setShortcutsOpen(true));
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
  const [, updateSetupProgress] = useState(0);
  useEffect(() => {
    const update = () => updateSetupProgress((value) => value + 1);
    window.addEventListener(FIRST_RUN_PROGRESS_EVENT, update);
    return () => window.removeEventListener(FIRST_RUN_PROGRESS_EVENT, update);
  }, []);
  const [workflow, setWorkflow] = useTabSheetState<WriteWorkflow>(
    'write.workflow',
    () =>
      fixtures
        ? initialWriteWorkflow(
            typeof window === 'undefined' ? '' : window.location.search,
            agentSnapshot,
          )
        : null,
    (value) => value?.kind === 'new' || value?.kind === 'new-folder',
    locations,
  );
  const catalog = useCatalogRuntime({
    lifetime,
    bridge,
    agentSnapshot,
    retireBoot,
    currentBootSnapshot,
    toasts,
  });
  const {
    latest,
    setLatest,
    agentCatalogReady,
    refreshingSnapshot,
    hardwareRefresh,
    refreshSnapshot,
    reloadCatalog,
    refresh,
    refreshAll,
  } = catalog;
  const runtime = useShellRuntime({
    lifetime,
    bridge,
    agentController,
    maintenanceOwnership,
    catalog,
    retireBoot,
    awaitBootRead,
    locations,
    locationKind: state.location.kind,
    toasts,
    setWorkflow,
    leaseClock,
  });
  const {
    agentLifecycle,
    concealSignal,
    setConcealSignal,
    commandError,
    recoverAgentReadiness,
    handleAgentReadinessFailure,
    reconciliation,
  } = runtime;
  const mutationError = useMutationError(commandError, catalog);
  // A team read that answers "this profile's vault has not been refreshed"
  // asks for that profile's catalog job, so the read that follows can find
  // the team again instead of waiting for the next scheduled refresh.
  const requestCatalog = useCallback(
    (profile: string) => reconciliation.invalidate(profile),
    [reconciliation],
  );
  const {
    observedExpiredLeases,
    accessGenerations,
    accessSession,
    accessNow,
    chatClock,
  } = useAccessRuntime({
    lifetime,
    agentSnapshot,
    latest,
    bridge,
    leaseClock,
    foregroundRefreshAllowed: runtime.foregroundRefreshAllowed,
    reconciliationRef: runtime.reconciliationRef,
    refreshSnapshotRef: runtime.refreshSnapshotRef,
    commandErrorRef: catalog.commandErrorRef,
  });
  const [resumeDraft, setResumeDraft] = useState<ResumeDraft | null>(null);
  const [revealRequest, setRevealRequest] = useState<string | null>(() =>
    scene.reveal && initialSelection
      ? `${initialSelection.store}|${initialSelection.path}`
      : null,
  );
  // Captured once because the canonical URL rewrite removes fixture-only intent.
  const [namedState] = useState(() =>
    !fixtures || typeof window === 'undefined'
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
      fixtures && scene.lease === 'lapsed'
        ? applyLease(reconciled, 'lapsed')
        : reconciled;
    const demonstrated = fixtures
      ? demoAvailabilityFacts(leased, namedState)
      : leased;
    if (fixtureFirstRunBoot) {
      return {
        ...demonstrated,
        agent: { state: 'bootstrap' as const, step: 'create-state' },
      };
    }
    return demonstrated;
  }, [
    fixtures,
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
  // A move made from a sheet settles its Recent and Favorites entries once
  // the refreshed snapshot holds the item under its new dirent id.
  useEffect(() => followMoves(shown.items), [shown.items]);
  const trash = useTrashActions({
    bridge,
    shown,
    refresh,
    refreshSnapshot,
    mutationError,
    toasts,
    locations,
    setWorkflow,
    accessNow,
  });
  const uploadDroppedFile = useDroppedUpload({
    bridge,
    shown,
    refresh,
    mutationError,
    setWorkflow,
  });
  const { prompt, settlePrompt, swipeIndicator } = useShellNavigation({
    locations,
    state,
    lease: scene.lease,
    toasts,
    blocked: shellChrome(
      shellBlock(agentLifecycle),
      agentLifecycle.state,
      shown.agent.state,
    ).blocked,
  });
  const here = state.location;
  const [teamInfo, setTeamInfo] = useState<{
    storeId: string;
    trigger?: HTMLElement;
  } | null>(null);
  useEffect(() => {
    setTeamInfo(null);
  }, [state.location, state.folder, state.selection, concealSignal]);
  useEffect(() => {
    if (state.details) setTeamInfo(null);
  }, [state.details]);
  const infoStore = teamInfo ? storeOf(shown, teamInfo.storeId) : undefined;
  useEffect(() => {
    if (teamInfo && infoStore?.kind !== 'team') setTeamInfo(null);
  }, [infoStore, teamInfo]);

  const pendingFirstRun = incompleteFirstRunCheckpoint();
  const detailsShown =
    state.details &&
    listsItems(here) &&
    !(here.kind === 'store' && !storeReadable(shown, here.ref)) &&
    !(state.selection && !storeReadable(shown, state.selection.store));
  // Where the header's New control saves, and whether it can. The items
  // screen settles the same destination from the same tree selection; this
  // reads that selection rather than a second source of truth, and falls back
  // to the account the screen would fall back to.
  const filesHere = listsItems(here);
  const newSelection = filesHere ? folderSelection(here, state.folder) : null;
  const newStore =
    newSelection === null
      ? undefined
      : (storeOf(
          shown,
          newSelection.store === ALL_ITEMS
            ? (defaultCreateStore(shown) ?? '')
            : newSelection.store,
        ) ?? undefined);
  const { sideCollapsed, railCollapsed, toggleSidebar, windowChromeHidden } =
    useWindowRuntime({
      bridge,
      locations,
      here,
      commandError,
    });
  const selectedAccessGeneration = state.selection
    ? (accessGenerations.get(
        storeOf(shown, state.selection.store)?.server ?? '',
      ) ?? 0)
    : 0;
  const { cache: deviceCache, devices: deviceMetadata } = useMetadataRuntime({
    lifetime,
    bridge,
    shown,
    concealSignal,
    accessGenerations,
    metadataInvalidation: catalog.metadataInvalidation,
    metadataReconciliation: catalog.metadataReconciliation,
    deviceRefresh: catalog.deviceRefresh,
    foregroundRefreshAllowed: runtime.foregroundRefreshAllowed,
    refreshSnapshot,
    report: commandError,
    initialDeviceCache,
  });
  // The timing log listens to the services that already report timings,
  // for as long as this shell and those services live.
  useEffect(
    () =>
      subscribeDiagnostics({
        scheduler: reconciliation.scheduler,
        workOwner: bridge,
        coordinator: catalog.catalogCoordinator,
        repository: deviceCache.repository,
      }),
    [
      reconciliation,
      bridge,
      catalog.catalogCoordinator,
      deviceCache.repository,
    ],
  );
  // The backend keeps its own timing log; Copy diagnostics reads it through
  // the log as one more source while the popover is open.
  useEffect(() => {
    const source = backendTimingSource(bridge);
    return source && diagnosticLog.addSource(source);
  }, [bridge]);
  // The rail's Teams badge reads the shared request-count rows, one per
  // named team this account can manage, loaded on unlock and kept by the
  // repository the shell provides to its pages, so the Teams list and a
  // team's page read the same rows. Devices' dot reads the recovery-phrase facts
  // maintained by the shell's shared metadata subscriptions. See
  // `device-alert.ts`.
  const teamRequestCounts = useTeamRequestCounts(
    bridge,
    shown,
    commandError,
    deviceCache.repository,
  );
  // The attention centre also lists each team's interrupted membership
  // changes, from the one journal row per profile a team's page reads.
  const pendingMembership = usePendingMembershipCounts(
    bridge,
    shown,
    commandError,
    deviceCache.repository,
  );
  const deviceAlerts = useSyncExternalStore(
    deviceAlertRegistry(bridge).subscribe,
    deviceAlertRegistry(bridge).getSnapshot,
  );
  // When each recovery phrase was last tested on this device: an untested or
  // overdue phrase is one of the Devices dot's reasons.
  const phraseTests = useSyncExternalStore(
    subscribeRecoveryPhraseTests,
    recoveryPhraseTests,
  );
  // The attention centre restates the rail's signals one entity at a time,
  // and adds the setup steps and the toast history. See `attention.ts`.
  const toastHistory = useSyncExternalStore(
    toasts.subscribeHistory,
    toasts.getHistory,
  );
  const [appLock, setAppLock] = useState<AppLockState | null>(null);
  useEffect(() => {
    let live = true;
    Promise.resolve()
      .then(() => bridge.appLockState())
      .then(
        (state) => {
          if (live) setAppLock(state);
        },
        // The checklist leaves the step out while the state is unknown.
        () => undefined,
      );
    return () => {
      live = false;
    };
  }, [bridge]);
  // Whether a recovery phrase is due for a test depends on the time. The
  // Devices dot and the bell's list read the same clock on the same render,
  // so a snooze that ends or a test that falls due moves both at once.
  const alertsAt = Date.now();
  const attentionList = attentionItems({
    snapshot: shown,
    teamRequests: teamRequestCounts,
    pendingMembership,
    devices: deviceAlerts,
    phraseTests,
    notices: unroutedNotices(shown),
    now: alertsAt,
  });
  const checklist = useMemo(
    () =>
      setupChecklist({
        snapshot: shown,
        devices: deviceAlerts,
        ...(appLock ? { appLock } : {}),
        catalogReady: agentCatalogReady,
      }),
    [agentCatalogReady, appLock, deviceAlerts, shown],
  );
  const appRef = useRef<HTMLDivElement>(null);
  const portalRoot = useMemo(
    () =>
      typeof document === 'undefined'
        ? null
        : (document.getElementById('overlays') ?? document.body),
    [],
  );
  const lockFromMenu = useCallback(() => {
    void onLock().then(
      (locked) => {
        if (!locked)
          toasts.show('Application lock is not available on this system.');
      },
      (error: unknown) => commandError(error),
    );
  }, [commandError, onLock, toasts]);

  // Render takeover overlays as siblings of the main grid so they remain
  // interactive while the grid is inert. The adjacent rail and topbar use the
  // same disabled styling as `BlockedShell`.
  const block = shellBlock(agentLifecycle);
  const chrome = shellChrome(block, agentLifecycle.state, shown.agent.state);
  // New works on a Files page whose store this account can write, from ⌘N,
  // the native menu and the palette's Actions alike.
  const newItemStore =
    !chrome.blocked &&
    filesHere &&
    newStore &&
    !writeBlockReason(shown, newStore)
      ? newStore
      : null;
  const startNewItem = useCallback(
    (itemKind: Exclude<KindFilter, 'All'>): boolean => {
      if (!newItemStore) return false;
      setWorkflow({
        kind: 'new',
        itemKind,
        storeId: newItemStore.id,
        // Nothing is created in Trash; New saves at the vault's top level.
        ...(newSelection &&
        newSelection.path !== '/' &&
        !isTrashPath(newSelection.path)
          ? { initialFolder: newSelection.path }
          : {}),
      });
      return true;
    },
    [newItemStore, newSelection, setWorkflow],
  );
  // Import starts where New would save: the same vault and folder.
  const startImport = useCallback((): boolean => {
    if (!newItemStore) return false;
    setWorkflow({
      kind: 'import',
      storeId: newItemStore.id,
      ...(newSelection &&
      newSelection.path !== '/' &&
      !isTrashPath(newSelection.path)
        ? { initialFolder: newSelection.path }
        : {}),
    });
    return true;
  }, [newItemStore, newSelection, setWorkflow]);
  // The shortcut and the menu do not open a sheet over another dialog; the
  // palette closes itself before its action runs.
  const openNewItem = useCallback(
    (itemKind: Exclude<KindFilter, 'All'>): boolean =>
      !anyDialogOpen() && startNewItem(itemKind),
    [startNewItem],
  );
  const paletteActions = useMemo(
    (): SearchAction[] => [
      ...(newItemStore
        ? [
            {
              id: 'new-password',
              label: 'New password',
              detail: `A login, API key or any secret value in ${newItemStore.name}`,
              icon: 'key' as const,
              keys: shortcutLabel('newPassword'),
              run: () => void startNewItem('Password'),
            },
            {
              id: 'new-document',
              label: 'Add a document',
              detail: `A file or a text value in ${newItemStore.name}`,
              icon: 'upload' as const,
              keys: shortcutLabel('newDocument'),
              run: () => void startNewItem('Document'),
            },
            {
              id: 'import-passwords',
              label: 'Import passwords',
              detail: `Logins from a CSV export of another password manager, into ${newItemStore.name}`,
              icon: 'download' as const,
              run: () => void startImport(),
            },
          ]
        : []),
      ...(filesHere &&
      newStore &&
      !chrome.blocked &&
      exportedPasswords(shown, newStore.id)
        ? [
            {
              id: 'export-passwords',
              label: 'Export passwords',
              detail: `The passwords in ${newStore.name}, as a plain-text CSV file`,
              icon: 'upload' as const,
              run: () => setWorkflow({ kind: 'export', storeId: newStore.id }),
            },
          ]
        : []),
      {
        id: 'lock',
        label: 'Lock',
        detail: 'Lock the app now',
        icon: 'lock',
        keys: shortcutLabel('lock'),
        run: lockFromMenu,
      },
      {
        id: 'shortcuts',
        label: 'Keyboard shortcuts',
        detail: 'Every shortcut and where it applies',
        icon: 'keyboard',
        keys: shortcutLabel('shortcuts'),
        run: () => setShortcutsOpen(true),
      },
      {
        id: 'settings',
        label: 'Settings',
        detail: 'Account, preferences and storage',
        icon: 'settings',
        run: () => locations.navigate({ kind: 'settings' }),
      },
      {
        id: 'add-device',
        label: 'Add a device',
        detail:
          'Pair a device, create a recovery phrase or connect a hardware key',
        icon: 'laptop',
        run: () => locations.navigate({ kind: 'devices', open: 'add' }),
      },
    ],
    [
      chrome.blocked,
      filesHere,
      lockFromMenu,
      locations,
      newItemStore,
      newStore,
      setWorkflow,
      shown,
      startImport,
      startNewItem,
    ],
  );
  // The palette row's item and the access problem a row's Copy would report
  // for it, or undefined for an item no longer listed.
  const paletteItem = useCallback(
    (storeId: string, path: string) => {
      const item = shown.items.find(
        (candidate) => candidate.store === storeId && candidate.path === path,
      );
      return item
        ? { item, problem: itemActionProblem(shown, item, false, accessNow()) }
        : undefined;
    },
    [accessNow, shown],
  );
  // ⌘C in the palette: the same checks and messages as a row's Copy.
  const copyFromPalette = useCallback(
    (storeId: string, path: string, field: ItemField): void => {
      const target = paletteItem(storeId, path);
      if (!target) return;
      void copyAndAnnounce({
        bridge,
        ...target,
        field,
        toasts,
        onError: commandError,
      });
    },
    [bridge, commandError, paletteItem, toasts],
  );
  // ⌘O in the palette: the same access check as a copy.
  const openWebsiteFromPalette = useCallback(
    (storeId: string, path: string): void => {
      const target = paletteItem(storeId, path);
      if (!target) return;
      void openWebsiteAndAnnounce({
        bridge,
        ...target,
        toasts,
        onError: commandError,
      });
    },
    [bridge, commandError, paletteItem, toasts],
  );
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      const addDocument = matchesShortcut(event, 'newDocument');
      if (
        (!addDocument && !matchesShortcut(event, 'newPassword')) ||
        document.querySelector('[role="menu"], [role="listbox"]')
      )
        return;
      if (openNewItem(addDocument ? 'Document' : 'Password'))
        event.preventDefault();
    };
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, [openNewItem]);
  // Lock works from any page and over any open sheet, whose unsaved contents
  // the lock discards; ⌘L types nothing, so a text field gives up nothing to
  // it. The macOS menu's Lock is handled in `useAppBootstrap`, which is
  // mounted whatever the window shows.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      if (!matchesShortcut(event, 'lock')) return;
      event.preventDefault();
      if (!event.repeat) lockFromMenu();
    };
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, [lockFromMenu]);
  useEffect(() => {
    if (!bridge.native) return;
    let disposed = false;
    let stop: (() => void) | undefined;
    void bridge
      .onNewItem((kind) => {
        if (!document.querySelector('[role="menu"], [role="listbox"]'))
          openNewItem(kind);
      })
      .then((unlisten) => {
        if (disposed) unlisten();
        else stop = unlisten;
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      stop?.();
    };
  }, [bridge, openNewItem]);
  const chatMigration = useChatMigration(
    bridge,
    agentLifecycle.state === 'ready' && agentCatalogReady && !chrome.blocked,
    concealSignal,
  );
  // A panel that failed to render has already unmounted, discarding anything
  // it revealed. Closing it keeps the shell from mounting it again, and the
  // toast states why it closed.
  const panelFault = (name: string, close: () => void) => (): void => {
    close();
    toasts.show(`${name} could not be shown and was closed.`, {
      tone: 'warning',
    });
  };
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
          enabled={
            agentLifecycle.state === 'ready' &&
            agentCatalogReady &&
            chatMigration.ready
          }
          bridge={bridge}
          snapshot={shown}
          onNavigate={navigateFromNotification}
          clock={chatClock}
          accessNow={accessNow}
          accessGenerations={accessGenerations}
          onCatalogRequired={bridge.native ? requestCatalog : undefined}
        >
          <div
            ref={swipeIndicator}
            className="history-swipe"
            hidden
            aria-hidden="true"
          />
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
              devicesAlert={devicesAlertSummary(
                shown,
                deviceAlerts,
                phraseTests,
                alertsAt,
              )}
            />
          ) : (
            <>
              <Sidebar
                snapshot={shown}
                location={here}
                account={locations.getAccount()}
                attention={unroutedNotices(shown).length}
                teamRequests={teamRequestsBadge(shown, teamRequestCounts)}
                devicesAlert={devicesAlertSummary(
                  shown,
                  deviceAlerts,
                  phraseTests,
                  alertsAt,
                )}
                settingsAlert={settingsAlertSummary(shown)}
                onTabNavigate={(tab) => locations.navigateTab(tab)}
                onNavigate={(location) => {
                  locations.navigate(location);
                }}
                onLock={lockFromMenu}
                // Setup progress is the rail's card, not a row: the card
                // names what comes next and continues where setup stopped.
                setup={
                  pendingFirstRun &&
                  completedFirstRunSteps(pendingFirstRun) <
                    firstRunStepCount(pendingFirstRun)
                    ? {
                        done: completedFirstRunSteps(pendingFirstRun),
                        total: firstRunStepCount(pendingFirstRun),
                        next: firstRunNextStep(pendingFirstRun),
                        onContinue: () =>
                          locations.navigate({
                            kind: 'first-run',
                            step: pendingFirstRun.state,
                            path: pendingFirstRun.path,
                          }),
                      }
                    : undefined
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
                  // One field for the shell: on the views that list items it
                  // drives the same query the browser reads.
                  {...(filesHere || here.kind === 'chat'
                    ? {
                        query: state.query,
                        onQuery: (query: string) => locations.search(query),
                      }
                    : {})}
                  {...(here.kind === 'chat'
                    ? {
                        onNewChat: requestNewChat,
                        onFindInChannel: requestChatFind,
                        chatTeamCount: chatTeams(shown).length,
                      }
                    : {})}
                  {...(filesHere && newStore
                    ? {
                        onNew: (itemKind: Exclude<KindFilter, 'All'>) =>
                          void openNewItem(itemKind),
                        onImport: () => {
                          if (!anyDialogOpen()) startImport();
                        },
                        newBlocked: writeBlockReason(shown, newStore),
                      }
                    : {})}
                  detailsOpen={detailsShown}
                  {...(filesHere
                    ? {
                        onToggleDetails: () => {
                          if (detailsShown) locations.setDetails(false);
                          else if (state.selection) locations.setDetails(true);
                        },
                      }
                    : {})}
                  collapsed={sideCollapsed}
                  onToggleCollapsed={toggleSidebar}
                  refreshing={refreshingSnapshot}
                  onRefresh={() =>
                    refreshAll(() => {
                      reconciliation.scheduler.requestAll('manual', [
                        'discovery',
                        'metadata',
                        'registry',
                        'connectivity',
                      ]);
                    }, commandError)
                  }
                  syncService={bridge.native ? reconciliation : undefined}
                  onOpenServers={(profile) =>
                    locations.navigate({
                      kind: 'settings',
                      section: 'account',
                      profile,
                    })
                  }
                  attention={{
                    items: attentionList,
                    checklist,
                    recent: toastHistory,
                    onNavigate: (location) => locations.navigate(location),
                    onClearRecent: () => toasts.clearHistory(),
                  }}
                />
                {chatMigration.notice}
                {/* Reset screen failures when the location changes while keeping
                    the rail and topbar outside the error boundary. */}
                <ScreenErrorBoundary
                  key={screenBoundaryKey(here)}
                  identity={screenIdentity(here)}
                >
                  <ScreenRouter
                    shown={shown}
                    bridge={bridge}
                    state={state}
                    locations={locations}
                    enteredScene={enteredScene}
                    namedState={namedState}
                    concealSignal={concealSignal}
                    hardwareRefresh={hardwareRefresh}
                    setDeviceLabel={setDeviceLabel}
                    setRevealRequest={setRevealRequest}
                    workflow={workflow}
                    setWorkflow={setWorkflow}
                    refresh={refresh}
                    refreshSnapshot={refreshSnapshot}
                    commandError={commandError}
                    mutationError={mutationError}
                    onLock={onLock}
                    agentLifecycle={agentLifecycle}
                    recoverAgentReadiness={recoverAgentReadiness}
                    uploadDroppedFile={uploadDroppedFile}
                    accessNow={accessNow}
                    accessGenerations={accessGenerations}
                    trash={trash}
                    onTeamInfo={(storeId, trigger) => {
                      locations.setDetails(false);
                      setTeamInfo({ storeId, trigger });
                    }}
                  />
                </ScreenErrorBoundary>
              </main>
            </>
          )}
          {!chrome.blocked &&
          listsItems(here) &&
          !detailsShown &&
          infoStore?.kind === 'team' ? (
            <PanelErrorBoundary
              label="team-info"
              onFault={panelFault('Team info', () => {
                setTeamInfo(null);
                teamInfo?.trigger?.focus({ preventScroll: true });
              })}
            >
              <TeamInfoPanel
                snapshot={shown}
                store={infoStore}
                bridge={bridge}
                requestCount={teamRequestCounts.get(infoStore.id)}
                accessNow={accessNow}
                onError={commandError}
                onNavigate={(location) => locations.navigate(location)}
                onClose={(restoreFocus = true) => {
                  setTeamInfo(null);
                  if (restoreFocus)
                    teamInfo?.trigger?.focus({ preventScroll: true });
                }}
              />
            </PanelErrorBoundary>
          ) : null}
          {here.kind !== 'first-run' && detailsShown ? (
            <PanelErrorBoundary
              label="details"
              onFault={panelFault('The details panel', () => {
                setRevealRequest(null);
                locations.setDetails(false);
              })}
            >
              <DetailsPanel
                snapshot={shown}
                bridge={bridge}
                revealRequest={revealRequest}
                onRevealHandled={() => setRevealRequest(null)}
                selection={state.selection}
                onClose={() => {
                  locations.setDetails(false);
                }}
                onMove={(item) => setWorkflow({ kind: 'move', item })}
                onCopyToVault={(item) => setWorkflow({ kind: 'copy', item })}
                onTrash={trash.moveToTrash}
                onConflict={(item, draft, operation) =>
                  setWorkflow({ kind: 'conflict', item, draft, operation })
                }
                onApplied={refresh}
                onCommandError={commandError}
                onMutationError={mutationError}
                concealSignal={concealSignal}
                accessGeneration={selectedAccessGeneration}
                accessNow={accessNow}
                accessSession={accessSession}
                accessTicket={lifetime.capture(
                  state.selection
                    ? storeOf(shown, state.selection.store)?.server
                    : undefined,
                )}
                resumeDraft={resumeDraft}
              />
            </PanelErrorBoundary>
          ) : null}
          <PanelErrorBoundary
            label="search"
            identity={String(searchOpen)}
            onFault={panelFault('Search', () => setSearchOpen(false))}
          >
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
              onOpenFolder={(storeId, path) =>
                locations.navigateToFolder(
                  { kind: 'store', ref: storeId },
                  folderKey(true, storeId, path),
                )
              }
              onCopyItem={copyFromPalette}
              onOpenWebsite={openWebsiteFromPalette}
              actions={paletteActions}
            />
          </PanelErrorBoundary>
          <ShortcutSheet
            open={shortcutsOpen}
            onClose={() => setShortcutsOpen(false)}
          />
        </ChatInboxProvider>
      </div>
      <ShellOverlays
        shown={shown}
        bridge={bridge}
        accessNow={accessNow}
        workflow={workflow}
        setWorkflow={setWorkflow}
        refresh={refresh}
        commandError={commandError}
        mutationError={mutationError}
        refreshSnapshot={refreshSnapshot}
        reloadCatalog={reloadCatalog}
        restoreTrash={trash.restoreTo}
        setResumeDraft={setResumeDraft}
        setConcealSignal={setConcealSignal}
        setLatest={setLatest}
        toasts={toasts}
        locations={locations}
        prompt={prompt}
        settlePrompt={settlePrompt}
        block={block}
        recoverAgentReadiness={recoverAgentReadiness}
      />
    </div>
  );

  const withToasts = (
    <ToastProvider controller={toasts} portalRoot={portalRoot}>
      <NavigationGuardProvider store={locations}>
        <MetadataRepositoryContext.Provider value={deviceCache.repository}>
          <DeviceCacheContext.Provider value={deviceCache}>
            <DeviceMetadataContext.Provider value={deviceMetadata}>
              {shell}
            </DeviceMetadataContext.Provider>
          </DeviceCacheContext.Provider>
        </MetadataRepositoryContext.Provider>
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
