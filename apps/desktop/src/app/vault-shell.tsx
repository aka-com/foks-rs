import { TeamInfoPanel } from '../screens/team-info';
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from 'react';
import {
  anyDialogOpen,
  anyPopupOpen,
  OverlayProvider,
} from '/kit/overlay-primitives';
import { mountFocusKeeper, setFocusContext } from '/kit/focus-keeper';
import { ToastController, ToastProvider } from '/kit/toasts';
import type { AgentLifecycleController } from '../agent-lifecycle';
import type { Bridge } from '../bridge';
import { useChatMigration } from '../chat/migration';
import { ChatInboxProvider } from '../chat/inbox-provider';
import { DeviceCacheContext } from '../device-cache';
import { FIRST_RUN_PROGRESS_EVENT } from '../first-run-operations';
import {
  FIRST_RUN_CHECKPOINT_KEY,
  completedFirstRunSteps,
  decodeFirstRunCheckpoint,
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
  settingsAlertSummary,
  storeOf,
  storeReadable,
  type AgentSnapshot,
  type DeviceLabel,
} from '../model';
import { NavigationGuardProvider, useTabSheetState } from '../navigation-guard';
import { MetadataRepositoryContext } from '../query-hooks';
import {
  systemLeaseExpiryClock,
  type LeaseExpiryClock,
} from '../scheduling/lease-expiry';
import {
  deviceAlertRegistry,
  devicesAlertSummary,
} from '../screens/device-alert';
import { DetailsPanel } from '../screens/details-panel';
import { FirstRunExperience } from '../screens/first-run-screen';
import { unroutedNotices } from '../screens/account-section';
import { ALL_ITEMS, folderSelection, listsItems } from '../screens/scope';
import { teamRequestsBadge, useTeamRequestCounts } from '../operation-queries';
import {
  initialWriteWorkflow,
  writeBlockReason,
  type WriteWorkflow,
} from '../screens/write-workflows';
import { useSearchShortcut } from '../shell/search-palette';
import { Sidebar } from '../shell/sidebar';
import { Topbar, type TopbarProps } from '../shell/topbar';
import { BarActionsSlot, DetailsSlot } from '../shell/page-slots';
import { activityBadge } from '../activity';
import { FilesViewMenu } from '../shell/files-view-menu';
import { PlainSyncStatus, SyncControls } from '../shell/sync-status';
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
import { useShellRuntime } from './shell-runtime';
import { ShellSearch } from './shell-search';
import { itemActionProblem } from '../screens/store-access';
import { commandKey } from '/kit/keys';
import { useWindowRuntime } from './window-runtime';

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
  const onLock = useCallback(async () => {
    const locked = await requestLock();
    if (locked) lifetime.retire('lock');
    return locked;
  }, [lifetime, requestLock]);
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
  const [toasts] = useState(() => new ToastController());
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
  // The focus never rests on <body>: a control that goes away hands it to
  // its replacement or to the place's list (`kit/focus-keeper.ts`). Each
  // place is a focus context, so Back returns to the control left there.
  useEffect(() => mountFocusKeeper(), []);
  const focusContext = JSON.stringify([here, state.folder, state.kind]);
  useLayoutEffect(() => setFocusContext(focusContext), [focusContext]);
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
  const [setupLater, setSetupLater] = useState(false);
  // The elements a page portals its bar commands and its details column
  // into (`shell/page-slots.tsx`).
  const [barSlot, setBarSlot] = useState<HTMLElement | null>(null);
  const [detailsSlot, setDetailsSlot] = useState<HTMLElement | null>(null);
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
  const deviceAlerts = useSyncExternalStore(
    deviceAlertRegistry(bridge).subscribe,
    deviceAlertRegistry(bridge).getSnapshot,
  );
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

  // Render takeover overlays as siblings of the main grid so they remain
  // interactive while the grid is inert. The adjacent rail and topbar use the
  // same disabled styling as `BlockedShell`.
  const block = shellBlock(agentLifecycle);
  const chrome = shellChrome(block, agentLifecycle.state, shown.agent.state);
  const openNewItem = useCallback(
    (itemKind: Exclude<KindFilter, 'All'>): boolean => {
      if (
        chrome.blocked ||
        !filesHere ||
        !newStore ||
        writeBlockReason(shown, newStore) ||
        anyDialogOpen()
      )
        return false;
      setWorkflow({
        kind: 'new',
        itemKind,
        storeId: newStore.id,
        ...(newSelection && newSelection.path !== '/'
          ? { initialFolder: newSelection.path }
          : {}),
      });
      return true;
    },
    [chrome.blocked, filesHere, newSelection, newStore, setWorkflow, shown],
  );
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      if (
        !(event.metaKey || event.ctrlKey) ||
        event.altKey ||
        event.key.toLowerCase() !== 'n' ||
        anyPopupOpen()
      )
        return;
      if (openNewItem(event.shiftKey ? 'Document' : 'Password'))
        event.preventDefault();
    };
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, [openNewItem]);
  // ⌘⌫ deletes the selected item, through the same alert the item's menu
  // opens, and only while nothing else holds the keyboard (a text field, a
  // menu or a dialog) and the focus is on the Files list or its details: a
  // destructive key acts on the view that has focus. An item the account
  // cannot delete is refused with the reason, without opening the alert.
  const selectedItem =
    filesHere && state.selection
      ? shown.items.find(
          (item) =>
            item.store === state.selection?.store &&
            item.path === state.selection.path &&
            item.kind !== 'Folder',
        )
      : undefined;
  useEffect(() => {
    if (!selectedItem || chrome.blocked) return;
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key !== 'Backspace' || !commandKey(event) || event.altKey)
        return;
      const target = event.target;
      if (
        target instanceof Element &&
        target.closest('input, textarea, select, [contenteditable="true"]')
      )
        return;
      if (anyDialogOpen() || anyPopupOpen()) return;
      const focused = document.activeElement;
      if (
        focused &&
        focused !== document.body &&
        !focused.closest('.lpane, .details')
      )
        return;
      event.preventDefault();
      const problem = itemActionProblem(shown, selectedItem, true, accessNow());
      if (problem) {
        toasts.show(problem, { tone: 'warning' });
        return;
      }
      setWorkflow({ kind: 'delete', item: selectedItem });
    };
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, [accessNow, chrome.blocked, selectedItem, setWorkflow, shown, toasts]);
  useEffect(() => {
    if (!bridge.native) return;
    let disposed = false;
    let stop: (() => void) | undefined;
    void bridge
      .onNewItem((kind) => {
        if (!anyPopupOpen()) openNewItem(kind);
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
  const refreshShell = (): void =>
    refreshAll(() => {
      reconciliation.scheduler.requestAll('manual', [
        'discovery',
        'metadata',
        'registry',
        'connectivity',
      ]);
    }, commandError);
  const openServer = (profile: string): void =>
    locations.navigate({ kind: 'settings', section: 'account', profile });
  const chatMigration = useChatMigration(
    bridge,
    agentLifecycle.state === 'ready' && agentCatalogReady && !chrome.blocked,
    concealSignal,
  );
  // Files is a list beside its details: with nothing selected (or the
  // details closed) the details column says No Selection rather than going
  // away, so the list keeps its width.
  const teamInfoShown =
    !chrome.blocked && filesHere && !detailsShown && infoStore?.kind === 'team';
  // A store page whose store is gone is a page of its own (the unavailable
  // notice), not a list. One that cannot be opened keeps the list layout,
  // with its access notice in the list column and No Selection beside it.
  const herePage = here.kind === 'store' && !storeOf(shown, here.ref);
  const splitEmpty = filesHere && !herePage && !detailsShown && !teamInfoShown;
  // Chat draws this bar over its own resizable channel column, with the
  // column's add control passed in, so the conversation beside it can carry
  // a bar of its own; every other place draws it across the top of the panel.
  const renderTopbar = (extra?: Partial<TopbarProps>): ReactNode => (
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
      actionsSlot={setBarSlot}
      {...(filesHere ||
      here.kind === 'chat' ||
      here.kind === 'activity' ||
      here.kind === 'devices'
        ? {
            query: state.query,
            onQuery: (query: string) => locations.search(query),
          }
        : {})}
      {...(here.kind === 'chat'
        ? { chatTeamCount: chatTeams(shown).length }
        : {})}
      {...(filesHere && newStore
        ? {
            onNew: (itemKind: Exclude<KindFilter, 'All'>) =>
              void openNewItem(itemKind),
            onNewFolder: () =>
              setWorkflow({
                kind: 'new-folder',
                storeId: newStore.id,
                initialFolder:
                  newSelection && newSelection.store !== ALL_ITEMS
                    ? newSelection.path
                    : '/',
              }),
            newBlocked: writeBlockReason(shown, newStore),
          }
        : {})}
      {...(filesHere
        ? {
            actions: (
              <FilesViewMenu
                sort={state.sort}
                direction={state.sortDirection}
                onSort={(sort, direction) => locations.setSort(sort, direction)}
                locationSort={newSelection?.store === ALL_ITEMS}
                kind={state.kind}
                onKind={(kind) => locations.setKind(kind)}
                disabled={chrome.blocked}
              />
            ),
          }
        : {})}
      collapsed={sideCollapsed}
      onToggleCollapsed={toggleSidebar}
      nativeChrome={bridge.native}
      {...extra}
    />
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
      <div
        className={[
          'app',
          detailsShown ||
          splitEmpty ||
          here.kind === 'activity' ||
          here.kind === 'devices' ||
          (here.kind === 'first-run' && here.step === 'added')
            ? 'with-details'
            : '',
          // The setup steps replace the sidebar with one that has no toggle;
          // the checklist screens draw the sidebar itself and hide with it.
          railCollapsed ? 'no-side' : '',
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
              devicesAlert={devicesAlertSummary(shown, deviceAlerts)}
            />
          ) : (
            <>
              <Sidebar
                snapshot={shown}
                location={here}
                account={locations.getAccount()}
                attention={unroutedNotices(shown).length}
                teamRequests={teamRequestsBadge(shown, teamRequestCounts)}
                activity={activityBadge(shown, teamRequestCounts)}
                devicesAlert={devicesAlertSummary(shown, deviceAlerts)}
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
                  !setupLater &&
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
                        // Later puts the card away for this session; the
                        // checkpoint, and the card at the next launch, stay.
                        onLater: () => setSetupLater(true),
                      }
                    : undefined
                }
                onToggleCollapsed={toggleSidebar}
                collapsed={sideCollapsed}
                files={{
                  folder: state.folder,
                  kind: state.kind,
                  onOpen: (folder, kind) => locations.openFiles(folder, kind),
                  accessNow,
                  folderActions: {
                    onNewFolder: (storeId, initialFolder) =>
                      setWorkflow({
                        kind: 'new-folder',
                        storeId,
                        initialFolder,
                      }),
                    onMove: (item) => setWorkflow({ kind: 'move', item }),
                    // Team info is a panel beside the Files list, so it is
                    // offered only while the list is on screen.
                    ...(filesHere
                      ? {
                          onTeamInfo: (
                            storeId: string,
                            trigger?: HTMLElement,
                          ) => {
                            locations.setDetails(false);
                            setTeamInfo({ storeId, trigger });
                          },
                        }
                      : {}),
                  },
                }}
                // The lifecycle is the sidebar's own authority for whether
                // the agent is gone, whichever path raised the disconnect.
                agent={chrome.agent}
                statusLine={
                  bridge.native ? (
                    <SyncControls
                      snapshot={shown}
                      service={reconciliation}
                      refreshing={refreshingSnapshot}
                      blocked={chrome.blocked}
                      onRefresh={refreshShell}
                      onOpenServers={openServer}
                    />
                  ) : (
                    <PlainSyncStatus
                      refreshing={refreshingSnapshot}
                      blocked={chrome.blocked}
                      onRefresh={refreshShell}
                    />
                  )
                }
                nativeChrome={bridge.native}
                blocked={chrome.blocked}
              />
              <main className="main" tabIndex={-1}>
                {here.kind === 'chat' ? null : renderTopbar()}
                {chatMigration.notice}
                {/* Reset screen failures when the location changes while keeping
                    the rail and topbar outside the error boundary. */}
                <ScreenErrorBoundary
                  key={screenBoundaryKey(here)}
                  identity={screenIdentity(here)}
                >
                  <BarActionsSlot.Provider value={barSlot}>
                    <DetailsSlot.Provider value={detailsSlot}>
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
                        onTeamInfo={(storeId, trigger) => {
                          locations.setDetails(false);
                          setTeamInfo({ storeId, trigger });
                        }}
                        renderBar={renderTopbar}
                      />
                    </DetailsSlot.Provider>
                  </BarActionsSlot.Provider>
                </ScreenErrorBoundary>
              </main>
            </>
          )}
          {!chrome.blocked &&
          listsItems(here) &&
          !detailsShown &&
          infoStore?.kind === 'team' ? (
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
          ) : null}
          {splitEmpty ? (
            <section className="details details-empty" aria-label="Details">
              {chrome.blocked ? null : (
                <>
                  <div className="bar" data-tauri-drag-region="" />
                  <div className="empty">
                    <b>No Selection</b>
                  </div>
                </>
              )}
            </section>
          ) : null}
          {here.kind === 'activity' || here.kind === 'devices' ? (
            // Activity's and Devices' details column: the page portals the
            // selected entry's details here.
            <section
              ref={setDetailsSlot}
              className={`details ${here.kind}-details`}
              aria-label="Details"
            />
          ) : null}
          {here.kind !== 'first-run' && detailsShown ? (
            <DetailsPanel
              snapshot={shown}
              bridge={bridge}
              revealRequest={revealRequest}
              onRevealHandled={() => setRevealRequest(null)}
              selection={state.selection}
              // Clearing the selection is guarded, so unsaved edits in the
              // panel ask before they are discarded.
              onClose={() => locations.select(null, { closeDetails: true })}
              onMove={(item) => setWorkflow({ kind: 'move', item })}
              onOpenFolder={(storeId, path) =>
                locations.openFiles(`${storeId}|${path}`, 'All')
              }
              onOpenServer={openServer}
              onDelete={(item) => setWorkflow({ kind: 'delete', item })}
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
