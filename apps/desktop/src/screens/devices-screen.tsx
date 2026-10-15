/**
 * The Devices tab: one page per account, with no sub-navigation.
 *
 * Recovery devices and Security keys were two panes two levels down, then
 * three sections of one page; they are one list here, sorted by type. Every
 * row carries only what the agent returns — a name, a type, a key id,
 * whether it is this Mac — and each kind's destructive action carries the
 * rest in its typed confirmation. The three ways to add a key are one
 * chooser, and it is the only add control left on the page: a security key's
 * card operations live on that key's own page instead of behind a menu here.
 *
 * FOKS calls a recovery phrase a recovery phrase; this page calls the object a
 * recovery phrase and keeps the enrollment name the agent stores.
 */

import { inventoryReadiness } from '../model';
import { CollectionStatus } from '../components/collection-status';
import { useDeviceMetadata } from '../data/device-cache';
import { synchronizeApplied } from '../data/operation-outcome';
import { refreshConnectedCards } from './devices/hardware-controller';
import { useDeviceOperation } from './devices/use-device-operation';
import { useWorkflowAccess } from '../workflow-context';
import {
  workflowAvailability,
  workflowMessage,
} from '../model/workflow-availability';
import { FreshnessCaption } from '../components/metadata-status';
import { useTabSheetState } from '../navigation-guard';
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react';
import type { ReactNode } from 'react';
import { useToast } from '../../kit/toasts';
import type {
  AccountDevice,
  BackupEnrollment,
  YubiEnrollment,
} from '../bridge';
import {
  Band,
  Button,
  Chip,
  Icon,
  Inset,
  InsetRow,
  Notice,
  SectionLabel,
} from '../components';
import type { StoreRef, DeviceLabel, Item } from '../model';
import { accountStores, plural, usernameOf } from '../model';
import type {
  DevicesSheetIntent,
  Location,
  NavigateOptions,
} from '../location';
import { PageHeader } from '../shell/page-header';
import { useShellServices, useShellSnapshot } from '../shell-services';
import { AddDeviceSheet } from './devices/sheets/add-device-sheet';
import { EnrollSheet } from './devices/sheets/enroll-sheet';
import { PairSheet } from './devices/sheets/pair-sheet';
import { PhraseSheet } from './devices/sheets/phrase-sheet';
import { ProvisionSheet } from './devices/sheets/provision-sheet';
import { RemoveDeviceSheet } from './devices/sheets/remove-device-sheet';
import { RevokeBackupSheet } from './devices/sheets/revoke-backup-sheet';
import { RevokeSheet } from './devices/sheets/revoke-sheet';
import { YubiActionSheet } from './devices/sheets/yubi-action-sheet';
import type { SimpleYubiAction } from './devices/credential-actions';
import {
  deviceAt,
  deviceEntries,
  enrollmentForDevice,
  deviceIsCard,
} from '../data/device-model';
import type { DeviceEntry } from '../data/device-model';
import { UnavailableAccount } from './account-section';
import { deviceAlertRegistry } from '../data/device-alert';
import { RecoveryPhraseTestSheet } from './recovery-phrase-test-sheet';
import { useExposureReviews } from '../exposure-review';
import { ExposureReviewBands, ExposureReviewSheet } from './exposure-review';
import {
  RECOVERY_TEST_INTERVAL_DAYS,
  recoveryPhraseTestDue,
  recoveryPhraseTestStatus,
  recoveryPhraseTests,
  snoozeRecoveryPhraseReminder,
  subscribeRecoveryPhraseTests,
  testedDate,
} from '../recovery-phrase-tests';
import type { RecoveryPhraseTestStatus } from '../recovery-phrase-tests';
import { kindLabel, TestedChip } from './devices/device-labels';
import { DeviceDetail } from './devices/device-detail';

type Sheet =
  | 'add'
  | 'pair'
  | 'phrase'
  | 'enroll'
  | 'provision'
  | 'yubi'
  | 'revoke'
  | 'remove-device'
  | 'revoke-backup'
  | 'test-backup'
  | null;

/**
 * The page reads the snapshot, the bridge, the catalog refreshes and the error
 * handlers from the shell services.
 */
export interface DevicesScreenProps {
  location: Extract<Location, { kind: 'devices' }>;
  /** The named fixture scene this tab was entered at, captured once. */
  scene: string;
  /**
   * `force` marks the replacement this page makes on its own behalf — the
   * default route resolving to an account's exact `StoreRef` — which is the
   * page already on screen writing its own address and not a move any sheet
   * may answer for.
   */
  onNavigate: (location: Location, options?: NavigateOptions) => void;
  onDeviceLabel?: (label: DeviceLabel | null) => void;
  /** Re-probe connected cards independently of cached enrollment metadata. */
  hardwareRefresh?: number;
  /**
   * Opens an item in Files, from the review of what a removed device could
   * read. Without it, the review lists items without an Open action.
   */
  onOpenItem?: (item: Item) => void;
}

export function DevicesScreen({
  location,
  scene,
  onNavigate,
  onDeviceLabel,
  hardwareRefresh = 0,
  onOpenItem,
}: DevicesScreenProps): ReactNode {
  const snapshot = useShellSnapshot();
  const {
    bridge,
    refresh: onRefresh,
    refreshSnapshot: onRefreshSnapshot,
    commandError: onError,
    mutationError: onMutationError,
  } = useShellServices();
  // The shell canonicalizes the route on mount; the scene it was entered at
  // is what decides whether a sheet opens with the page.
  const [enteredScene] = useState(scene);
  const toasts = useToast();
  const stores = accountStores(snapshot);
  const requested = location.store;
  const selected = requested
    ? stores.find((store) => store.id === requested)
    : stores[0];
  const deviceAccess = workflowAvailability(snapshot, 'devices-list', {
    profile: selected?.server,
    account: selected?.account,
  });
  const stopped = {
    stopped: !deviceAccess.available,
    reason: workflowMessage(deviceAccess) ?? '',
  };
  const access = useWorkflowAccess(snapshot);
  const target = { profile: selected?.server, account: selected?.account };
  const canAdd = (
    ['device-pair', 'backup-create', 'yubi-provision'] as const
  ).some((operation) => access.availability(operation, target).available);
  const addReason = canAdd
    ? undefined
    : access.props('device-pair', target).title;
  const [pairMode, setPairMode] = useTabSheetState<'offer' | 'accept'>(
    'devices.pairMode',
    'offer',
  );
  const [sheet, setSheet] = useTabSheetState<Sheet>(
    'devices.sheet',
    () =>
      enteredScene === 'settings-phrase'
        ? 'phrase'
        : enteredScene === 'settings-enrol'
          ? 'enroll'
          : null,
    (value) => value === 'pair' && pairMode === 'accept',
  );
  const [cards, setCards] = useState<{ serial: number }[]>([]);
  const [pendingYubi, setPendingYubi] = useState<SimpleYubiAction | null>(null);
  const [removing, setRemoving] = useState<AccountDevice | null>(null);
  const [revoking, setRevoking] = useState<BackupEnrollment | null>(null);
  // Which enrollment a per-row key action acts on. The row is the only thing
  // that says which one; there may be more than one.
  const [actingKey, setActingKey] = useState<YubiEnrollment | null>(null);
  // Reviews of what removed devices could read, kept on this device after
  // the removal; one is open in its sheet at a time.
  const exposureReviews = useExposureReviews();
  const [exposureOpen, setExposureOpen] = useState<string | null>(null);
  const exposureReview = exposureReviews.find(
    (review) => review.id === exposureOpen && review.scope === selected?.id,
  );

  const sheetOpen = useRef(sheet);
  sheetOpen.current = sheet;
  const {
    cache: deviceCache,
    lists,
    loading,
    failed,
    backupsUnknown,
    freshness,
    retry: retryMetadata,
  } = useDeviceMetadata({
    bridge,
    snapshot,
    profile: selected?.server,
    store: selected?.id,
    enabled: !stopped.stopped,
    recovery: { refresh: onRefreshSnapshot, allowed: () => !sheetOpen.current },
    onError,
  });
  const { devices, backups, yubi } = lists;
  const devicesAlert = useSyncExternalStore(
    deviceAlertRegistry(bridge).subscribe,
    deviceAlertRegistry(bridge).getSnapshot,
  );
  const phraseTests = useSyncExternalStore(
    subscribeRecoveryPhraseTests,
    recoveryPhraseTests,
  );
  const testStatus = (backup: BackupEnrollment): RecoveryPhraseTestStatus =>
    recoveryPhraseTestStatus(phraseTests, backup.backupId);
  const testAccess = access.props('backup-list', target);
  const openTest = (): void => setSheet('test-backup');
  const pairingOfferOpen = selected
    ? devicesAlert.offers.has(selected.id)
    : false;

  const closeSheets = useCallback((): void => {
    setSheet(null);
    setPendingYubi(null);
    setRemoving(null);
    setRevoking(null);
    setActingKey(null);
    setExposureOpen(null);
  }, [setSheet]);

  // Normalize the route to the active account's StoreRef. Because the
  // destination matches the current location, navigation guards are bypassed
  // and active dialogs remain open.
  useEffect(() => {
    if (location.store || !selected) return;
    onNavigate({ ...location, store: selected.id }, { force: true });
  }, [location, onNavigate, selected]);

  const selectedId = selected?.id;
  const profile = selected?.server;
  const hardware = useDeviceOperation(JSON.stringify([profile, selectedId]));

  // Switching accounts drops what the page was doing with the last one. The
  // first read is not a switch: a scene may have opened a sheet with the page.
  const shown = useRef(selectedId);
  useEffect(() => {
    if (shown.current === selectedId) return;
    shown.current = selectedId;
    closeSheets();
  }, [closeSheets, selectedId]);

  // The address can arrive asking for Add a device (`open: 'add'`), which the
  // ⌘K palette's Actions send from any page, or for Create a recovery phrase
  // (`open: 'phrase'`), which the attention centre's setup step sends. As on
  // the Teams page, the intent is spent in two commits: the address is
  // replaced with the plain list first, and the sheet opens on the commit
  // after that, so the sheet's tab-persisted state belongs to the list's
  // address and Back, a re-render or a return to the tab lands on the list.
  const intent = location.open;
  const navigateRef = useRef(onNavigate);
  navigateRef.current = onNavigate;
  const requestedSheet = useRef<{
    store: StoreRef;
    intent: DevicesSheetIntent;
  } | null>(null);
  const canCreatePhrase = access.availability(
    'backup-create',
    target,
  ).available;
  const phraseReason = canCreatePhrase
    ? undefined
    : access.props('backup-create', target).title;
  useEffect(() => {
    // With no account yet the page cannot say whose device would be added,
    // so the intent waits for the catalog rather than being spent on nothing.
    if (!intent || !selectedId) return;
    requestedSheet.current = { store: selectedId, intent };
    navigateRef.current(
      { kind: 'devices', store: selectedId },
      { replace: true, force: true },
    );
  }, [intent, selectedId]);
  useEffect(() => {
    const request = requestedSheet.current;
    if (intent || !request) return;
    requestedSheet.current = null;
    if (request.store !== selectedId) return;
    const [allowed, reason] =
      request.intent === 'phrase'
        ? [canCreatePhrase, phraseReason]
        : [canAdd, addReason];
    if (allowed) setSheet(request.intent);
    else if (reason) toasts.show(reason, { tone: 'warning' });
  }, [
    intent,
    selectedId,
    canAdd,
    addReason,
    canCreatePhrase,
    phraseReason,
    setSheet,
    toasts,
  ]);

  // Card presence is live hardware state, never part of the metadata cache.
  useEffect(() => {
    setCards([...snapshot.cardsConnected]);
  }, [snapshot.cardsConnected, hardwareRefresh]);

  // A `section=` address — the Devices addresses written before the page was
  // one, when Macs and Security keys were two sections — lands on the one
  // list both now name.
  const listAnchor = useRef<HTMLDivElement>(null);
  const requestedSection = location.section;
  useEffect(() => {
    if (!requestedSection) return;
    // If an initial scene immediately opens a dialog (e.g., `settings-phrase`),
    // preserve focus within the dialog rather than shifting focus to the
    // background section anchor until the dialog is closed.
    if (sheet) return;
    const anchor = listAnchor.current;
    if (!anchor) return;
    if (typeof anchor.scrollIntoView === 'function')
      anchor.scrollIntoView({ block: 'start' });
    anchor.focus({ preventScroll: true });
  }, [requestedSection, sheet]);

  // `onRefresh` reloads the catalog and says what happened; the page does not
  // repeat the sentence.
  const applied = async (message: string): Promise<void> => {
    closeSheets();
    const resources =
      profile && selectedId
        ? [
            deviceCache.account(profile, selectedId),
            deviceCache.enrollments(profile),
          ]
        : [];
    const versions = resources.map((query) => query.getSnapshot().invalidation);
    const result = await synchronizeApplied(() => onRefresh(message, profile));
    if (result.synchronization === 'pending') {
      toasts.show(`${message} Refresh pending.`);
      onError(result.error);
    }
    // A forced catalog refresh may already have invalidated these resources.
    // Invalidation since the completed write is sufficient; do not start it twice.
    resources.forEach((query, index) => {
      if (query.getSnapshot().invalidation === versions[index])
        query.invalidate();
    });
  };
  const copyText = (text: string, said: string): void => {
    void bridge
      .copyText(text)
      .then(() => toasts.show(said))
      .catch(onError);
  };
  const copy = (text: string): void => copyText(text, 'Pairing phrase copied');
  /** One key's own page, addressed by the key it is about. */
  const openDevice = (address: string): void =>
    onNavigate({ ...location, device: address });
  /**
   * The list, at one account. Spreading the address instead would carry the
   * `device=` of the page it was written on, which names a key that account
   * does not hold.
   */
  const listAt = (at: StoreRef | undefined): Location => ({
    kind: 'devices',
    ...(location.section ? { section: location.section } : {}),
    ...(at ? { store: at } : {}),
  });
  const backToList = (): void => onNavigate(listAt(location.store));
  // An authenticated device may be a key on a card, which is not a Mac; the
  // count says so rather than calling every device one.
  const cardKeys = devices.filter((device) => deviceIsCard(device)).length;
  const subtitle = stopped.stopped
    ? 'Not listed while access is stopped'
    : loading
      ? // Counting what has not been read yet would report three zeroes.
        'Loading…'
      : [
          plural(devices.length - cardKeys, 'computer'),
          ...(cardKeys
            ? [plural(cardKeys, 'key on a card', 'keys on cards')]
            : []),
          plural(backups.length, 'recovery phrase'),
          // `list_yubi_accounts` answers with enrollments, not with keys: one
          // count per object, under the name that object carries everywhere.
          plural(yubi.length, 'enrollment'),
        ].join(' · ');
  const why = stopped.stopped ? stopped.reason : undefined;
  // A key's own page always names the enrollment its Card operations act on;
  // there is no page-wide "the one complete enrollment" left to fall back to.
  const openYubi = (action: SimpleYubiAction, entry: YubiEnrollment): void => {
    setActingKey(entry);
    setPendingYubi(action);
    setSheet('yubi');
  };
  // Every key on the account, sorted by type: computers, then recovery phrases,
  // then security key enrollments — the one list the page now shows.
  const entries = deviceEntries(lists);
  /** The row's trailing chip or button, ahead of the "Open" button every row
   * carries: which one depends on which of the three lists it came from. */
  const deviceRowAction = (entry: DeviceEntry): ReactNode => {
    const source = entry.source;
    if (source.kind === 'device') {
      const device = source.device;
      if (device.current) return null;
      if (deviceIsCard(device)) return <Chip tone="ok">Security key</Chip>;
      return (
        <Button
          size="sm"
          variant="danger"
          disabled={stopped.stopped}
          title={why}
          onClick={() => {
            setRemoving(device);
            setSheet('remove-device');
          }}
        >
          Remove…
        </Button>
      );
    }
    if (source.kind === 'backup') {
      const backup = source.backup;
      return (
        <>
          <TestedChip status={testStatus(backup)} />
          <Button size="sm" {...testAccess} onClick={openTest}>
            Test…
          </Button>
          <Button
            size="sm"
            variant="danger"
            {...access.props('backup-revoke', target)}
            onClick={() => {
              setRevoking(backup);
              setSheet('revoke-backup');
            }}
          >
            Revoke…
          </Button>
        </>
      );
    }
    const enrollment = source.entry;
    return (
      <>
        <Chip tone={enrollment.state === 'complete' ? 'ok' : 'warn'}>
          {enrollment.state === 'complete' ? 'Enrolled' : 'Incomplete'}
        </Chip>
        <Button
          size="sm"
          variant="danger"
          disabled={stopped.stopped || enrollment.state !== 'complete'}
          title={
            stopped.stopped
              ? stopped.reason
              : enrollment.state === 'complete'
                ? undefined
                : 'This enrollment is not complete'
          }
          onClick={() => {
            setActingKey(enrollment);
            setSheet('revoke');
          }}
        >
          Revoke…
        </Button>
      </>
    );
  };
  // The key a `device=` address names, once the four lists have answered.
  const detail = location.device
    ? deviceAt({ devices, backups, yubi, cards }, location.device)
    : undefined;

  const detailAddress = detail?.address;
  const detailName = detail?.name;
  useEffect(() => {
    onDeviceLabel?.(
      selectedId && detailAddress && detailName
        ? { store: selectedId, address: detailAddress, name: detailName }
        : null,
    );
    return () => onDeviceLabel?.(null);
  }, [onDeviceLabel, selectedId, detailAddress, detailName]);

  const accountReadiness = inventoryReadiness(snapshot, 'accounts');
  if (!selected && accountReadiness !== 'ready')
    return (
      <>
        <PageHeader ruled title="Devices" subtitle="" />
        <div className="body">
          <CollectionStatus state={accountReadiness} label="accounts" />
          <Button onClick={() => void onRefreshSnapshot().catch(onError)}>
            Refresh
          </Button>
        </div>
      </>
    );

  // An address naming an account this Mac no longer holds is reported as
  // exactly that, and the notice is the only list of the accounts it does
  // hold: a switcher beside it would offer the same accounts twice.
  // "No account configured" would be a different, false statement.
  if (!selected && requested)
    return (
      <>
        <PageHeader ruled title="Devices" subtitle="" />
        <div className="body">
          <div className="settings-main">
            <UnavailableAccount
              stores={stores}
              snapshot={snapshot}
              onSelect={(store) => onNavigate(listAt(store.id))}
              onRefresh={() => void onRefreshSnapshot().catch(onError)}
            />
          </div>
        </div>
      </>
    );

  // Devices are scoped by account. If no accounts exist locally, no devices
  // are displayed; new accounts must be added via Settings › Account.
  if (!selected)
    return (
      <>
        <PageHeader ruled title="Devices" subtitle="" />
        <div className="body">
          <div className="settings-main">
            <Notice
              title="No account configured"
              actions={
                <Button
                  variant="primary"
                  onClick={() =>
                    onNavigate({ kind: 'settings', section: 'account' })
                  }
                >
                  Open Account
                </Button>
              }
            >
              <p>
                Add or recover an account before configuring devices and
                security keys.
              </p>
            </Notice>
          </div>
        </div>
      </>
    );

  // Under the list it describes, never at the top of the page: a stale read
  // is a fact about the list, and the caption says so where the list is.
  const metadataStatus = (
    <FreshnessCaption
      label="device metadata"
      freshness={freshness}
      onRetry={retryMetadata}
    />
  );
  const staleClass = freshness.stale ? ' stale' : '';
  // A scan of the Mac's own ports. On the list it follows the rows it updates;
  // a key's own page keeps the same control available for card operations.
  const refreshHardwareKeys = (
    <Button
      size="sm"
      {...access.props('yubi-scan', { profile: selected.server })}
      onClick={() =>
        void refreshConnectedCards(
          bridge,
          access,
          selected.server,
          hardware.capture(),
          (found) => {
            setCards(found);
            // A scan that finds nothing is a legitimate result, not an
            // error, so the empty case has to say so itself.
            if (found.length === 0) toasts.show('No security keys found');
          },
          onError,
        )
      }
    >
      Refresh hardware keys
    </Button>
  );

  return (
    <>
      {/* One key's own page: the full id, and the one destructive action for
          that kind. It reads the page's own lists, so a sheet opened from it
          is the sheet the row would have opened. */}
      {location.device ? (
        <DeviceDetail
          snapshot={snapshot}
          store={selected}
          entry={detail}
          deviceEnrollment={
            detail?.source.kind === 'device'
              ? enrollmentForDevice(yubi, detail.source.device.id)
              : undefined
          }
          loading={loading}
          failed={failed}
          stopped={stopped}
          cards={cards}
          metadataStatus={metadataStatus}
          stale={freshness.stale}
          refreshHardwareKeys={refreshHardwareKeys}
          onBack={backToList}
          onNavigate={onNavigate}
          onCopy={(text) => copyText(text, 'Key id copied')}
          onRemove={(device) => {
            setRemoving(device);
            setSheet('remove-device');
          }}
          onRevokeBackup={(backup) => {
            setRevoking(backup);
            setSheet('revoke-backup');
          }}
          testStatus={testStatus}
          onTestBackup={openTest}
          onRevokeKey={(entry) => {
            setActingKey(entry);
            setSheet('revoke');
          }}
          onProvision={() => setSheet('provision')}
          onYubiAction={openYubi}
        />
      ) : (
        <>
          <PageHeader
            ruled
            title="Devices"
            subtitle={subtitle}
            action={
              <Button
                variant="primary"
                icon="plus"
                disabled={!canAdd}
                title={addReason}
                onClick={() => setSheet('add')}
              >
                Add a device
              </Button>
            }
          />
          <div className="body">
            <div className="settings-main">
              {stopped.stopped ? (
                <Band
                  severity="crit"
                  label="Account access is stopped"
                  action={
                    <Button
                      size="sm"
                      onClick={() =>
                        onNavigate({
                          kind: 'settings',
                          section: 'account',
                          profile: selected.server,
                        })
                      }
                    >
                      Open server
                    </Button>
                  }
                >
                  Devices and keys are not listed while access is stopped.
                </Band>
              ) : null}
              <ExposureReviewBands
                reviews={exposureReviews.filter(
                  (review) => review.scope === selected?.id,
                )}
                onReview={setExposureOpen}
              />
              {!stopped.stopped && pairingOfferOpen ? (
                <Band
                  label="A pairing is waiting on this device"
                  action={
                    <Button
                      size="sm"
                      onClick={() => {
                        setPairMode('offer');
                        setSheet('pair');
                      }}
                    >
                      Resume offer…
                    </Button>
                  }
                >
                  Type the phrase already issued on the other device, or open it
                  again here.
                </Band>
              ) : null}
              {!stopped.stopped &&
              !loading &&
              !failed &&
              !backupsUnknown &&
              !backups.length ? (
                <Band
                  label="No recovery key"
                  action={
                    <Button
                      size="sm"
                      {...access.props('backup-create', target)}
                      onClick={() => setSheet('phrase')}
                    >
                      Create a recovery phrase…
                    </Button>
                  }
                >
                  This account cannot be recovered on a new device.
                </Band>
              ) : null}
              {stopped.stopped || loading
                ? null
                : backups.map((backup) => {
                    const status = testStatus(backup);
                    if (!recoveryPhraseTestDue(status)) return null;
                    return (
                      <Band
                        key={backup.backupId}
                        label={
                          status.state === 'never'
                            ? 'Recovery phrase not tested'
                            : 'Recovery phrase due for a test'
                        }
                        action={
                          <>
                            <Button
                              size="sm"
                              onClick={() =>
                                snoozeRecoveryPhraseReminder(backup.backupId)
                              }
                            >
                              Remind me later
                            </Button>
                            <Button
                              size="sm"
                              {...testAccess}
                              onClick={openTest}
                            >
                              Test now
                            </Button>
                          </>
                        }
                      >
                        {status.state === 'never' ? (
                          <>
                            No written copy of {backup.backupAlias} has been
                            tested on this device. Test it against your copy so
                            a mistake in it is found before the copy is needed.
                          </>
                        ) : (
                          <>
                            {backup.backupAlias} was last tested on{' '}
                            {testedDate(status.testedAt)}, more than{' '}
                            {RECOVERY_TEST_INTERVAL_DAYS} days ago. Test it
                            against your written copy to make sure it can still
                            recover this account.
                          </>
                        )}
                      </Band>
                    );
                  })}

              <div
                className="settings-section"
                ref={listAnchor}
                tabIndex={-1}
                role="region"
                aria-labelledby="devices-all-label"
              >
                {/* Every key on the account is one list, sorted by type:
                    computers (and any device key on a card among them), then
                    recovery phrases, then security key enrollments — the order
                    `deviceEntries` already reads the three lists in. */}
                <SectionLabel id="devices-all-label">Devices</SectionLabel>
                <Inset className={`settings-inset middle wide${staleClass}`}>
                  {loading ? (
                    <InsetRow label="Devices">
                      Loading devices, recovery phrases and security keys…
                    </InsetRow>
                  ) : entries.length ? (
                    entries.map((entry) => (
                      <InsetRow
                        key={entry.address}
                        className="devrow"
                        onClick={(event) => {
                          if (
                            event.target instanceof Element &&
                            event.target.closest(
                              'button, a, input, select, textarea',
                            )
                          )
                            return;
                          openDevice(entry.address);
                        }}
                        action={
                          <>
                            {deviceRowAction(entry)}
                            <OpenDevice
                              name={entry.name}
                              onOpen={() => openDevice(entry.address)}
                            />
                          </>
                        }
                      >
                        <span className="device-icon" aria-hidden="true">
                          <Icon name={entry.icon} />
                        </span>
                        <span className="t">
                          <b>{entry.name}</b>
                          <small>{kindLabel(entry.kind)}</small>
                        </span>
                        {entry.source.kind === 'device' &&
                        entry.source.device.current ? (
                          <Chip tone="you">Current</Chip>
                        ) : null}
                      </InsetRow>
                    ))
                  ) : (
                    <InsetRow label="Devices">
                      {stopped.stopped
                        ? 'Not listed while access is stopped'
                        : failed
                          ? 'Devices and keys could not be read. Refresh to try again.'
                          : 'No devices, recovery phrases or security keys on this account.'}
                    </InsetRow>
                  )}
                </Inset>
                <div className="device-list-actions">
                  {refreshHardwareKeys}
                  {metadataStatus}
                </div>
              </div>
            </div>
          </div>
        </>
      )}
      {sheet === 'add' && selected ? (
        <AddDeviceSheet
          store={selected}
          onClose={() => setSheet(null)}
          onChoose={(choice) => {
            // The chooser names the direction, so accepting a phrase has an
            // entry point again: both pairing cards land on the pair sheet.
            if (choice === 'pair' || choice === 'pair-accept')
              setPairMode(choice === 'pair' ? 'offer' : 'accept');
            setSheet(choice === 'pair-accept' ? 'pair' : choice);
          }}
        />
      ) : null}
      {sheet === 'pair' && selected ? (
        <PairSheet
          store={selected}
          initialMode={pairMode}
          onCopy={copy}
          onBack={() => setSheet('add')}
          onClose={() => setSheet(null)}
          onDone={async (message) => applied(message)}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
      {sheet === 'phrase' && selected ? (
        <PhraseSheet
          profile={selected.server}
          accountAlias={selected.account}
          seedPhrase={
            enteredScene === 'settings-phrase'
              ? bridge.firstRunFixture?.recoveryPhrase
              : undefined
          }
          onClose={() => setSheet(null)}
          onDone={async () => {
            await applied('Recovery phrase created.');
          }}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
      {sheet === 'enroll' && selected ? (
        <EnrollSheet
          store={selected}
          card={cards[0]}
          onClose={() => setSheet(null)}
          onDone={async () => applied('Hardware-key account created.')}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
      {sheet === 'provision' && selected ? (
        <ProvisionSheet
          store={selected}
          cards={cards}
          onClose={() => setSheet(null)}
          onDone={async () => applied('Hardware key added to account.')}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
      {sheet === 'yubi' && selected && pendingYubi ? (
        <YubiActionSheet
          store={selected}
          action={pendingYubi}
          // The row or Card operations button this was opened from names its
          // own enrollment; `openYubi` never opens this sheet without one.
          alias={actingKey?.alias ?? ''}
          onClose={() => {
            setSheet(null);
            setPendingYubi(null);
            setActingKey(null);
          }}
          onDone={async () => {
            setPendingYubi(null);
            await applied('Security key updated.');
          }}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
      {sheet === 'revoke' && selected && actingKey ? (
        <RevokeSheet
          store={selected}
          alias={actingKey.alias}
          onClose={() => {
            setSheet(null);
            setActingKey(null);
          }}
          onDone={async () => {
            // The page this was started from is about a key that no longer
            // exists, so the list is where the reader is left.
            if (location.device) backToList();
            await applied('Hardware key revoked.');
          }}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
      {sheet === 'remove-device' && selected && removing ? (
        <RemoveDeviceSheet
          store={selected}
          device={removing}
          onClose={() => {
            setSheet(null);
            setRemoving(null);
          }}
          onDone={async () => {
            const name = removing.name ?? 'device';
            setRemoving(null);
            if (location.device) backToList();
            await applied(`Device "${name}" removed`);
          }}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
      {exposureReview ? (
        <ExposureReviewSheet
          snapshot={snapshot}
          review={exposureReview}
          onOpenItem={
            onOpenItem
              ? (item) => {
                  setExposureOpen(null);
                  onOpenItem(item);
                }
              : undefined
          }
          onClose={() => setExposureOpen(null)}
        />
      ) : null}
      {sheet === 'test-backup' && selected ? (
        <RecoveryPhraseTestSheet
          store={selected}
          username={usernameOf(snapshot, selected) ?? selected.account}
          backups={backups}
          onClose={() => setSheet(null)}
          onError={onError}
        />
      ) : null}
      {sheet === 'revoke-backup' && selected && revoking ? (
        <RevokeBackupSheet
          store={selected}
          backup={revoking}
          onClose={() => {
            setSheet(null);
            setRevoking(null);
          }}
          onDone={async () => {
            const alias = revoking.backupAlias;
            setRevoking(null);
            if (location.device) backToList();
            await applied(`Revoked recovery phrase ${alias}`);
          }}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
    </>
  );
}

/** The chevron at the end of a key's row: that key's own page. */
function OpenDevice({
  name,
  onOpen,
}: {
  name: string;
  onOpen: () => void;
}): ReactNode {
  return (
    <Button size="sm" aria-label={`Open ${name}`} onClick={onOpen}>
      Open
    </Button>
  );
}
