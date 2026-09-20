import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from 'react';
import { OverlayProvider } from '/kit/overlay-primitives';
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
  firstRunStepCount,
  type FirstRunCheckpoint,
} from '../first-run-state';
import {
  storeAtScene,
  useLocationState,
  type LocationStore,
} from '../location';
import {
  applyLease,
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
import {
  FirstRunChecklistStatus,
  FirstRunExperience,
} from '../screens/first-run-screen';
import { unroutedNotices } from '../screens/account-section';
import { listsItems } from '../screens/scope';
import { teamRequestsBadge, useTeamRequestCounts } from '../operation-queries';
import {
  initialWriteWorkflow,
  type WriteWorkflow,
} from '../screens/write-workflows';
import { useSearchShortcut } from '../shell/search-palette';
import { Sidebar } from '../shell/sidebar';
import { Topbar } from '../shell/topbar';
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
    (value) => value?.kind === 'new',
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
  const { prompt, settlePrompt } = useShellNavigation({
    locations,
    state,
    lease: scene.lease,
    toasts,
  });
  const here = state.location;
  const pendingFirstRun = incompleteFirstRunCheckpoint();
  const detailsShown =
    state.details &&
    listsItems(here) &&
    !(here.kind === 'store' && !storeReadable(shown, here.ref)) &&
    !(state.selection && !storeReadable(shown, state.selection.store));
  const { sideCollapsed, railCollapsed, toggleSidebar, windowChromeHidden } =
    useWindowRuntime({
      bridge,
      locations,
      here,
      detailsShown,
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
  // team's page read the same rows. Devices' dot reads the paper-key facts
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
  const chatMigration = useChatMigration(
    bridge,
    agentLifecycle.state === 'ready' && agentCatalogReady && !chrome.blocked,
    concealSignal,
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
                  />
                </ScreenErrorBoundary>
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
