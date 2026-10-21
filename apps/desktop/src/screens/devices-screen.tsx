import { inventoryReadiness } from '../model';
import { CollectionStatus } from '../components/collection-status';
import { useDeviceMetadata } from '../device-cache';
import { synchronizeApplied } from '../operation-outcome';
import { refreshConnectedCards } from './devices/hardware-controller';
import { useDeviceOperation } from './devices/use-device-operation';
import { WorkflowProvider, useWorkflowAccess } from '../workflow-context';
import {
  workflowAvailability,
  workflowMessage,
} from '../model/workflow-availability';
import { FreshnessCaption } from '../components/metadata-status';
import { useTabSheetState } from '../navigation-guard';
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

import {
  useCallback,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react';
import type { ReactNode } from 'react';
import { useToast } from '/kit/toasts';
import { ContextMenu, Menu } from '/kit/overlay-primitives';
import type {
  AccountDevice,
  BackupEnrollment,
  Bridge,
  YubiEnrollment,
} from '../bridge';
import {
  Band,
  Button,
  Chip,
  CopyBox,
  Icon,
  Inset,
  InsetRow,
  MenuButton,
  MenuItem,
  Notice,
  SectionLabel,
  useListbox,
} from '../components';
import { BarActions, DetailsColumn } from '../shell/page-slots';
import type {
  AccountStore,
  AgentSnapshot,
  StoreRef,
  DeviceLabel,
} from '../model';
import {
  accountStores,
  serverDisplayLabelForStore,
  usernameOf,
} from '../model';
import type { Location, NavigateOptions } from '../location';
import type { MutationFailureHandler } from '../mutation-recovery';
import {
  AddDeviceSheet,
  EnrollSheet,
  PairSheet,
  PhraseSheet,
  ProvisionSheet,
  RemoveDeviceSheet,
  RevokeBackupSheet,
  RevokeSheet,
  YubiActionSheet,
  YUBI_WORKFLOWS,
} from './device-sheets';
import type { SimpleYubiAction } from './device-sheets';
import { deviceAt, deviceEntries, enrollmentForDevice } from './device-model';
import type { DeviceEntry } from './device-model';
import { UnavailableAccount } from './account-section';

import { deviceAlertRegistry } from './device-alert';

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
  | null;

/**
 * The nine card operations that are recovery paths for a key already in
 * trouble — what used to be the "More…" menu's `RECOVERY_ACTIONS`, one menu
 * entry per action. Each is now a button on the key's own page instead, and
 * the sheet it opens reads the same label either way.
 */
type RecoveryAction =
  | 'sync'
  | 'set-passphrase'
  | 'change-passphrase'
  | 'verify-passphrase'
  | 'recover-management'
  | 'recover-subkey'
  | 'resume-enrollment'
  | 'resume-rotation'
  | 'rotate';

/**
 * The rows "Card operations" groups the nine `RecoveryAction`s into, on a
 * security key's own page: Sync (1) + Passphrase (3) + Recovery (5) = 9,
 * plus the Connected row's "Provision…" and "PIN status" — the eleven
 * operations the old "More…" menu and Connected row together held.
 */
const CARD_OP_GROUPS: ReadonlyArray<{
  label: string;
  hint: string;
  actions: readonly RecoveryAction[];
}> = [
  {
    label: 'Sync',
    hint: 'Refresh this account’s state on the card.',
    actions: ['sync'],
  },
  {
    label: 'Passphrase',
    hint: 'Set, change or verify the key passphrase.',
    actions: ['set-passphrase', 'change-passphrase', 'verify-passphrase'],
  },
  {
    label: 'Recovery',
    hint: 'Restore access if the card, its management key or its signing key is stuck.',
    actions: [
      'recover-management',
      'recover-subkey',
      'resume-enrollment',
      'resume-rotation',
      'rotate',
    ],
  },
];

/** The short label a `CARD_OP_GROUPS` button reads; the row it sits under
 * already carries the rest of the sentence. */
const CARD_OP_LABELS: Readonly<Record<RecoveryAction, string>> = {
  sync: 'Sync…',
  'set-passphrase': 'Set…',
  'change-passphrase': 'Change…',
  'verify-passphrase': 'Verify…',
  'recover-management': 'Restore Management…',
  'recover-subkey': 'Restore Signing Key…',
  'resume-enrollment': 'Resume Enrollment…',
  'resume-rotation': 'Resume Rotation…',
  rotate: 'Rotate…',
};

/**
 * The word a row's caption, and a key's own "Kind" fact, name its type with,
 * on this page only. The shared model still calls a card enrollment an
 * "Enrollment" object — People reads that literal value unchanged — but
 * Devices calls the same object a security key everywhere else in its copy.
 */
function kindLabel(kind: DeviceEntry['kind']): string {
  return kind === 'Enrollment' ? 'Security key' : kind;
}

/**
 * The list's groups and the sources each one holds. A key on a card that the
 * account authenticates as a device is a device; a card enrollment is a
 * security key.
 */
const DEVICE_GROUPS: ReadonlyArray<
  readonly [string, ReadonlyArray<DeviceEntry['source']['kind']>]
> = [
  ['Devices', ['device']],
  ['Security keys', ['yubi']],
  ['Recovery phrases', ['backup']],
];

/** A row's caption: its type, its role, and a security key's card serial. */
function deviceCaption(entry: DeviceEntry): string {
  const serial =
    entry.source.kind === 'yubi' ? entry.source.entry.cardSerial : undefined;
  return [kindLabel(entry.kind), entry.role, serial ? String(serial) : '']
    .filter(Boolean)
    .join(' · ');
}

export interface DevicesScreenProps {
  snapshot: AgentSnapshot;
  bridge: Bridge;
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
  /**
   * Refreshes data after a mutation. Device operations are scoped to the
   * selected account's server profile.
   */
  onRefresh: (message: string, profile?: string) => Promise<void>;
  onRefreshSnapshot: () => Promise<AgentSnapshot>;
  onError: (error: unknown) => void;
  onMutationError: MutationFailureHandler;
  onDeviceLabel?: (label: DeviceLabel | null) => void;
  /** Re-probe connected cards independently of cached enrollment metadata. */
  hardwareRefresh?: number;
  /** The bar's search text: the list narrows to the names it matches. */
  query?: string;
}

export function DevicesScreen({
  snapshot,
  bridge,
  location,
  scene,
  onNavigate,
  onRefresh,
  onRefreshSnapshot,
  onError,
  onMutationError,
  onDeviceLabel,
  hardwareRefresh = 0,
  query = '',
}: DevicesScreenProps): ReactNode {
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
  // A device row's context menu (right click, the ContextMenu key or
  // Shift-F10): the row's copy and destructive actions, as its page has them.
  const [rowMenu, setRowMenu] = useState<{
    x: number;
    y: number;
    entry: DeviceEntry;
  } | null>(null);

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
  const pairingOfferOpen = selected
    ? devicesAlert.offers.has(selected.id)
    : false;

  const closeSheets = useCallback((): void => {
    setSheet(null);
    setPendingYubi(null);
    setRemoving(null);
    setRevoking(null);
    setActingKey(null);
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
  /**
   * A row's context menu: Show Details, Copy for the key's id, and the
   * destructive action its page offers, with the same reasons. A key on a
   * card and this Mac have none here, as on their pages.
   */
  const deviceRowMenuItems = (
    entry: DeviceEntry,
    close: () => void,
  ): ReactNode => {
    const source = entry.source;
    const cardKey = source.kind === 'device' && entry.kind === 'Key on a card';
    const removable = !cardKey && !entry.current;
    const revokeReason =
      access.props('yubi-revoke', target).title ??
      (source.kind === 'yubi' && source.entry.state !== 'complete'
        ? 'This enrollment is not complete'
        : undefined);
    return (
      <>
        <MenuItem
          icon="chevronRight"
          onClick={() => {
            close();
            openDevice(entry.address);
          }}
        >
          Show Details
        </MenuItem>
        {entry.keyId ? (
          <MenuItem
            icon="copy"
            onClick={() => {
              close();
              if (entry.keyId) copyText(entry.keyId, 'Key id copied');
            }}
          >
            {entry.source.kind === 'backup'
              ? 'Copy Backup ID'
              : 'Copy Device Key'}
          </MenuItem>
        ) : null}
        {removable ? <div className="menu-separator" role="separator" /> : null}
        {removable && source.kind === 'device' ? (
          <MenuItem
            icon="trash"
            danger
            reason={access.props('device-remove', target).title}
            onClick={() => {
              close();
              setRemoving(source.device);
              setSheet('remove-device');
            }}
          >
            Remove This Device…
          </MenuItem>
        ) : null}
        {removable && source.kind === 'backup' ? (
          <MenuItem
            icon="trash"
            danger
            reason={access.props('backup-revoke', target).title}
            onClick={() => {
              close();
              setRevoking(source.backup);
              setSheet('revoke-backup');
            }}
          >
            Revoke Recovery Phrase…
          </MenuItem>
        ) : null}
        {removable && source.kind === 'yubi' ? (
          <MenuItem
            icon="trash"
            danger
            reason={revokeReason}
            onClick={() => {
              close();
              setActingKey(source.entry);
              setSheet('revoke');
            }}
          >
            Revoke Enrollment…
          </MenuItem>
        ) : null}
      </>
    );
  };
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
  /** The row's trailing chip: this Mac, or a security key's enrollment
   * state. A row's destructive action is on the key's details. */
  const deviceRowChip = (entry: DeviceEntry): ReactNode => {
    if (entry.current) return <Chip tone="you">This device</Chip>;
    if (entry.source.kind !== 'yubi') return null;
    const complete = entry.source.entry.state === 'complete';
    return (
      <Chip tone={complete ? 'ok' : 'warn'}>
        {complete ? 'Enrolled' : 'Incomplete'}
      </Chip>
    );
  };
  // The list narrows to the names the bar's search field matches.
  const needle = query.trim().toLowerCase();
  const matching = needle
    ? entries.filter((entry) => entry.name.toLowerCase().includes(needle))
    : entries;
  // In the order the list draws them: by group, then as `deviceEntries` reads
  // each group.
  const shownEntries = DEVICE_GROUPS.flatMap(([, sources]) =>
    matching.filter((entry) => sources.includes(entry.source.kind)),
  );
  // The rows are one listbox: one Tab stop, Up/Down/Home/End and
  // type-select. Selection follows the keyboard; once a key is open, moving
  // to the next one replaces its address rather than stacking a history entry
  // per row, so Back still leaves Devices in one step.
  const rows = useListbox({
    keys: shownEntries.map((entry) => entry.address),
    selectedKey: location.device ?? null,
    labelOf: (address) =>
      shownEntries.find((entry) => entry.address === address)?.name ?? '',
    onMove: (address) =>
      onNavigate(
        { ...location, device: address },
        location.device ? { replace: true } : undefined,
      ),
  });
  const firstSecurityKey = shownEntries.find(
    (entry) => entry.source.kind === 'yubi',
  )?.address;
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
        <section className="lpane devices-list" aria-label="Devices">
          <div className="body rows">
            <div className="list-head">
              <div className="t">
                <h2>Devices</h2>
              </div>
            </div>
          </div>
        </section>
        <DetailsColumn>
          <div className="bar" data-tauri-drag-region="" />
          <div className="dbody">
            <div className="dinner">
              <CollectionStatus state={accountReadiness} label="accounts" />
              <Button onClick={() => void onRefreshSnapshot().catch(onError)}>
                Refresh
              </Button>
            </div>
          </div>
        </DetailsColumn>
      </>
    );

  // An address naming an account this Mac no longer holds is reported as
  // exactly that, and the notice is the only list of the accounts it does
  // hold: a switcher beside it would offer the same accounts twice.
  // "No account configured" would be a different, false statement.
  if (!selected && requested)
    return (
      <>
        <section className="lpane devices-list" aria-label="Devices">
          <div className="body rows">
            <div className="list-head">
              <div className="t">
                <h2>Devices</h2>
              </div>
            </div>
          </div>
        </section>
        <DetailsColumn>
          <div className="bar" data-tauri-drag-region="" />
          <div className="dbody">
            <div className="dinner">
              <UnavailableAccount
                stores={stores}
                snapshot={snapshot}
                onSelect={(store) => onNavigate(listAt(store.id))}
                onRefresh={() => void onRefreshSnapshot().catch(onError)}
              />
            </div>
          </div>
        </DetailsColumn>
      </>
    );

  // Devices are scoped by account. If no accounts exist locally, no devices
  // are displayed; new accounts must be added via Settings › Account.
  if (!selected)
    return (
      <>
        <section className="lpane devices-list" aria-label="Devices">
          <div className="body rows">
            <div className="list-head">
              <div className="t">
                <h2>Devices</h2>
              </div>
            </div>
          </div>
        </section>
        <DetailsColumn>
          <div className="bar" data-tauri-drag-region="" />
          <div className="dbody">
            <div className="dinner">
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
        </DetailsColumn>
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
  // A scan of the Mac's own ports: the ⋯ menu in the bar offers it for the
  // list, and a key that is not listed offers it beside its notice.
  const scan = access.props('yubi-scan', { profile: selected.server });
  const scanReason = scan.disabled ? (scan.title ?? 'Unavailable') : undefined;
  const refreshHardware = (): void =>
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
    );
  const refreshHardwareKeys = (
    <Button size="sm" {...scan} onClick={refreshHardware}>
      Refresh Hardware Keys
    </Button>
  );

  return (
    <WorkflowProvider snapshot={snapshot}>
      {/* The panel's bar carries the list's commands: the hardware scan, in
          the ⋯ menu, and Add a Device. */}
      <BarActions>
        <MenuButton
          label=""
          aria-label="More"
          title="More"
          variant="quiet"
          icon="ellipsis"
          trailingIcon={null}
          menuLabel="Devices"
          align="end"
        >
          {(close) => (
            <MenuItem
              icon="refresh"
              reason={scanReason}
              onClick={() => {
                close();
                refreshHardware();
              }}
            >
              Refresh Hardware Keys
            </MenuItem>
          )}
        </MenuButton>
        <Button
          variant="quiet"
          icon="plus"
          aria-label="Add a device"
          disabled={!canAdd}
          title={addReason ?? 'Add a device'}
          onClick={() => setSheet('add')}
        />
      </BarActions>
      <section
        className="lpane devices-list"
        ref={listAnchor}
        tabIndex={-1}
        role="region"
        aria-labelledby="devices-all-label"
      >
        <div className="body rows">
          <div className="list-head">
            <div className="t">
              <h2 id="devices-all-label">Devices</h2>
              <p>
                {usernameOf(snapshot, selected) ?? selected.account} ·{' '}
                {serverDisplayLabelForStore(snapshot, selected)}
              </p>
            </div>
          </div>
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
                  Open Server
                </Button>
              }
            >
              Devices and keys are not listed while access is stopped.
            </Band>
          ) : null}
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
                  Resume Offer…
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
                  Create a Recovery Phrase…
                </Button>
              }
            >
              This account cannot be recovered on a new device.
            </Band>
          ) : null}
          {loading ? (
            <p className="list-note" role="status">
              Loading devices, recovery phrases and security keys…
            </p>
          ) : entries.length ? (
            /* Account devices (computers, and keys on cards authenticated as
               devices), then security key enrollments, then recovery phrases,
               each group in the order `deviceEntries` reads it. */
            <>
              <div
                {...rows.listProps}
                className={`device-groups${staleClass}`}
                aria-labelledby="devices-all-label"
              >
                {DEVICE_GROUPS.map(([group, sources]) => {
                  const groupRows = shownEntries.filter((entry) =>
                    sources.includes(entry.source.kind),
                  );
                  if (!groupRows.length) return null;
                  return (
                    <div key={group} role="group" aria-label={group}>
                      <div className="gsec" aria-hidden="true">
                        {group}
                      </div>
                      {groupRows.map((entry) => {
                        const on = entry.address === location.device;
                        return (
                          <button
                            type="button"
                            key={entry.address}
                            {...rows.optionProps(entry.address)}
                            className={on ? 'row devrow sel' : 'row devrow'}
                            aria-label={entry.name}
                            onClick={() => openDevice(entry.address)}
                            onContextMenu={(event) => {
                              event.preventDefault();
                              event.currentTarget.focus();
                              setRowMenu({
                                x: event.clientX,
                                y: event.clientY,
                                entry,
                              });
                            }}
                            onKeyDown={(event) => {
                              if (
                                event.key !== 'ContextMenu' &&
                                !(event.key === 'F10' && event.shiftKey)
                              )
                                return;
                              event.preventDefault();
                              const bounds =
                                event.currentTarget.getBoundingClientRect();
                              setRowMenu({
                                x: bounds.left,
                                y: bounds.bottom,
                                entry,
                              });
                            }}
                          >
                            <span className="tile" aria-hidden="true">
                              <Icon name={entry.icon} />
                            </span>
                            <span className="name">
                              <b>{entry.name}</b>
                              <small>{deviceCaption(entry)}</small>
                            </span>
                            <span className="r">{deviceRowChip(entry)}</span>
                          </button>
                        );
                      })}
                    </div>
                  );
                })}
              </div>
              {query && !shownEntries.length ? (
                <p className="list-note">
                  No device, recovery phrase or security key matches “
                  {query.trim()}”.
                </p>
              ) : null}
            </>
          ) : (
            <p className="list-note">
              {stopped.stopped
                ? 'Not listed while access is stopped'
                : failed
                  ? 'Devices and keys could not be read. Refresh to try again.'
                  : 'No devices, recovery phrases or security keys on this account.'}
            </p>
          )}
          <div className="device-list-actions">{metadataStatus}</div>
        </div>
      </section>
      {rowMenu ? (
        <ContextMenu
          point={rowMenu}
          className="menu-portal"
          onClose={() => setRowMenu(null)}
        >
          <Menu
            className="menu"
            aria-label={`Actions for ${rowMenu.entry.name}`}
            initialFocus="first"
            onClose={() => setRowMenu(null)}
          >
            {deviceRowMenuItems(rowMenu.entry, () => setRowMenu(null))}
          </Menu>
        </ContextMenu>
      ) : null}
      {/* One key's details beside the list: the full id, and the one
          destructive action for that kind. It reads the page's own lists, so
          a sheet opened from it is the sheet the row would have opened. */}
      <DetailsColumn>
        <div className="bar" data-tauri-drag-region="" />
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
            onRevokeKey={(entry) => {
              setActingKey(entry);
              setSheet('revoke');
            }}
            onProvision={() => setSheet('provision')}
            onYubiAction={openYubi}
            onShowSecurityKeys={
              firstSecurityKey
                ? () => rows.focusKey(firstSecurityKey)
                : undefined
            }
          />
        ) : (
          <div className="empty">
            <b>No Selection</b>
          </div>
        )}
      </DetailsColumn>
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
          bridge={bridge}
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
          bridge={bridge}
          profile={selected.server}
          accountAlias={selected.account}
          subtitle={`For ${usernameOf(snapshot, selected) ?? selected.account} on ${serverDisplayLabelForStore(snapshot, selected)}`}
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
          bridge={bridge}
          store={selected}
          card={cards[0]}
          onClose={() => setSheet(null)}
          onDone={async () => applied('Hardware-key account created.')}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
      {sheet === 'provision' && selected ? (
        <ProvisionSheet
          bridge={bridge}
          store={selected}
          cards={cards}
          onClose={() => setSheet(null)}
          onDone={async () => applied('Hardware key added to account.')}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
      {sheet === 'yubi' && selected && pendingYubi ? (
        <YubiActionSheet
          bridge={bridge}
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
          bridge={bridge}
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
          bridge={bridge}
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
      {sheet === 'revoke-backup' && selected && revoking ? (
        <RevokeBackupSheet
          bridge={bridge}
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
    </WorkflowProvider>
  );
}

/**
 * One key's page: every fact the agent reports about it, the full id with a
 * way to copy it, and the one destructive action its kind has.
 */
function DeviceDetail({
  snapshot,
  store,
  entry,
  deviceEnrollment,
  loading,
  failed,
  stopped,
  cards,
  stale,
  refreshHardwareKeys,
  onBack,
  onNavigate,
  onCopy,
  onRemove,
  onRevokeBackup,
  onRevokeKey,
  onProvision,
  onYubiAction,
  onShowSecurityKeys,
}: {
  snapshot: AgentSnapshot;
  store: AccountStore;
  /** The key the address names, once the lists have answered. */
  entry?: DeviceEntry;
  deviceEnrollment?: YubiEnrollment;
  loading: boolean;
  /** The lists never answered, so whether the key is listed is unknown. */
  failed: boolean;
  stopped: { stopped: boolean; reason: string };
  /** The cards in this Mac's ports right now. */
  cards: { serial: number }[];
  /** The lists this page reads could not be refreshed. */
  stale: boolean;
  refreshHardwareKeys: ReactNode;
  onBack: () => void;
  onNavigate: (location: Location) => void;
  onCopy: (text: string) => void;
  onRemove: (device: AccountDevice) => void;
  onRevokeBackup: (backup: BackupEnrollment) => void;
  onRevokeKey: (entry: YubiEnrollment) => void;
  onProvision: () => void;
  onYubiAction: (action: SimpleYubiAction, entry: YubiEnrollment) => void;
  /** Moves focus to the list's first security key; absent when it has none. */
  onShowSecurityKeys?: () => void;
}): ReactNode {
  const access = useWorkflowAccess(snapshot);
  const target = { profile: store.server, account: store.account };
  if (!entry)
    return (
      <div className="dbody devices-detail">
        <div className="dinner">
          <Notice
            severity={
              loading || stopped.stopped ? 'info' : failed ? 'warn' : 'crit'
            }
            title={
              stopped.stopped
                ? 'Not listed while access is stopped'
                : loading
                  ? 'Reading this account’s keys…'
                  : failed
                    ? 'Devices and keys could not be read'
                    : 'This key is not on this account'
            }
            actions={
              <>
                <Button variant="primary" onClick={onBack}>
                  Back to Devices
                </Button>
                {refreshHardwareKeys}
              </>
            }
          >
            <p>
              {stopped.stopped
                ? stopped.reason
                : loading
                  ? 'Loading devices, recovery phrases, and security keys…'
                  : failed
                    ? 'Whether this key is still on this account is unknown until its devices, recovery phrases and security keys are read again.'
                    : 'This key is no longer associated with this account on this device.'}
            </p>
          </Notice>
        </div>
      </div>
    );

  const source = entry.source;
  const username = usernameOf(snapshot, store) ?? store.account;
  const enrollment = source.kind === 'yubi' ? source.entry : undefined;
  const revokeReason =
    access.props('yubi-revoke', target).title ??
    (enrollment && enrollment.state !== 'complete'
      ? 'This enrollment is not complete'
      : undefined);
  const caption =
    entry.scope === 'profile'
      ? `${kindLabel(entry.kind)} · on ${serverDisplayLabelForStore(snapshot, store)}`
      : kindLabel(entry.kind);
  // A key on a card is the card's, whether or not this Mac is authenticated
  // with it now: it is not removed the way another Mac is, so its kind decides
  // this before its currency does. Neither it nor this Mac has a destructive
  // action here; each says where its removal is instead, as a note.
  const cardKey = source.kind === 'device' && entry.kind === 'Key on a card';
  return (
    <div className="dbody devices-detail">
      <div className="dinner">
        <div className="hero">
          <span className="bigtile" aria-hidden="true">
            <Icon name={entry.icon} />
          </span>
          <div className="t">
            <h1>{entry.name}</h1>
            {/* The chip states what this key is, so it reads beside the
                caption rather than among actions. An enrollment is the
                server's, not this account's, and the caption says so. */}
            <p>
              {caption}
              {entry.current ? (
                <>
                  {' · '}
                  <Chip tone="you">
                    {entry.kind === 'Key on a card'
                      ? 'Current key on a card'
                      : 'This device'}
                  </Chip>
                </>
              ) : enrollment ? (
                <>
                  {' · '}
                  <Chip tone={enrollment.state === 'complete' ? 'ok' : 'warn'}>
                    {enrollment.state === 'complete'
                      ? 'Enrolled'
                      : 'Incomplete'}
                  </Chip>
                </>
              ) : null}
            </p>
          </div>
        </div>
        {cardKey ? (
          <Notice
            severity="info"
            title="Revoked under its enrollment"
            actions={
              // The enrollment is a row in this same list: open the one that
              // matches, or else take focus to the Security keys rows.
              deviceEnrollment ? (
                <Button
                  onClick={() =>
                    onNavigate({
                      kind: 'devices',
                      section: 'keys',
                      store: store.id,
                      device: `yubi:${deviceEnrollment.alias}`,
                    })
                  }
                >
                  {`Open ${deviceEnrollment.alias}`}
                </Button>
              ) : onShowSecurityKeys ? (
                <Button onClick={onShowSecurityKeys}>Show Security Keys</Button>
              ) : undefined
            }
          >
            <p>
              A key on a card is revoked under its enrollment, listed under
              Security keys.
            </p>
          </Notice>
        ) : entry.current ? (
          <Notice
            severity="info"
            title="This device cannot be removed here"
            actions={
              <Button
                onClick={() => onNavigate({ kind: 'settings', section: 'mac' })}
              >
                Open Settings
              </Button>
            }
          >
            <p>
              To remove this device, use another enrolled device. Or, use the
              Reset this device option in Settings to erase local data and
              delete credentials across servers.
            </p>
          </Notice>
        ) : null}
        <div
          className="settings-section"
          role="region"
          aria-labelledby="device-facts-label"
        >
          <SectionLabel className="offscreen" id="device-facts-label">
            This key
          </SectionLabel>
          <Inset
            className={`settings-inset middle wide${stale ? ' stale' : ''}`}
          >
            {entry.scope === 'profile' ? (
              <InsetRow label="Server">
                <b>{serverDisplayLabelForStore(snapshot, store)}</b>
                <small>This enrollment applies across the server.</small>
              </InsetRow>
            ) : (
              <InsetRow label="Account">
                <span className="namechip">
                  <b>{username}</b> <Chip>{store.account}</Chip>
                </span>
                <small>on {serverDisplayLabelForStore(snapshot, store)}</small>
              </InsetRow>
            )}
            {entry.role ? (
              <InsetRow label="Role on the account">{entry.role}</InsetRow>
            ) : null}
            <InsetRow label="Kind">{kindLabel(entry.kind)}</InsetRow>
            <InsetRow label={entry.keyLabel}>
              {entry.keyId ? (
                <CopyBox text={entry.keyId} onCopy={onCopy}>
                  <span className="mono">{entry.keyId}</span>
                </CopyBox>
              ) : (
                <>
                  {entry.name}
                  <small>
                    {enrollment?.cardSerial
                      ? `Card serial ${enrollment.cardSerial}; no device key recorded.`
                      : 'This enrollment cannot be matched to a card on this device.'}
                  </small>
                </>
              )}
            </InsetRow>
            {entry.current ? (
              <InsetRow label="Status">
                Authenticated on this device now
              </InsetRow>
            ) : null}
          </Inset>
        </div>
        {enrollment ? (
          <CardOperations
            enrollment={enrollment}
            cards={cards}
            stopped={stopped}
            store={store}
            onNavigate={onNavigate}
            onProvision={onProvision}
            onYubiAction={onYubiAction}
          />
        ) : null}
        {cardKey || entry.current ? null : (
          <div
            className="settings-section"
            role="region"
            aria-labelledby="device-danger-label"
          >
            <SectionLabel className="danger-title" id="device-danger-label">
              Danger zone
            </SectionLabel>
            <Inset className="settings-inset middle wide danger-box">
              {source.kind === 'device' ? (
                <InsetRow
                  className="dangerrow"
                  label="Remove this device"
                  action={
                    <Button
                      size="sm"
                      variant="danger"
                      {...access.props('device-remove', target)}
                      onClick={() => onRemove(source.device)}
                    >
                      Remove This Device…
                    </Button>
                  }
                >
                  <small>
                    {entry.name} loses access to every vault, team and chat on
                    this account. Data already downloaded to it stays there.
                  </small>
                </InsetRow>
              ) : source.kind === 'backup' ? (
                <InsetRow
                  className="dangerrow"
                  label="Revoke this recovery phrase"
                  action={
                    <Button
                      size="sm"
                      variant="danger"
                      {...access.props('backup-revoke', target)}
                      onClick={() => onRevokeBackup(source.backup)}
                    >
                      Revoke…
                    </Button>
                  }
                >
                  <small>
                    The written-down phrase stops working, and revocation
                    rotates every account key it could read.
                  </small>
                </InsetRow>
              ) : (
                <InsetRow
                  className="dangerrow"
                  label="Revoke this enrollment"
                  action={
                    <Button
                      size="sm"
                      variant="danger"
                      disabled={revokeReason !== undefined}
                      title={revokeReason}
                      onClick={() => onRevokeKey(source.entry)}
                    >
                      Revoke…
                    </Button>
                  }
                >
                  <small>
                    Disconnects the key this enrollment was made on and updates
                    account security.
                  </small>
                </InsetRow>
              )}
            </Inset>
          </div>
        )}
      </div>
    </div>
  );
}

/**
 * The maintenance operations a security key's card supports, grouped under
 * one heading on the enrollment's own page: what used to be the "More…" menu
 * on the list, plus "Provision…" and "PIN status", which used to be their own
 * buttons on a connected card's row. Every button here acts on this page's
 * own enrollment — there is no other one to guess at.
 */
function CardOperations({
  enrollment,
  cards,
  store,
  onNavigate,
  onProvision,
  onYubiAction,
}: {
  enrollment: YubiEnrollment;
  /** The cards in this Mac's ports right now. */
  cards: { serial: number }[];
  stopped: { stopped: boolean; reason: string };
  store: AccountStore;
  onNavigate: (location: Location) => void;
  onProvision: () => void;
  onYubiAction: (action: SimpleYubiAction, entry: YubiEnrollment) => void;
}): ReactNode {
  const access = useWorkflowAccess();
  const target = { profile: store.server, account: enrollment.alias };
  const why = access.props('yubi-provision', {
    profile: store.server,
    account: store.account,
  }).title;
  const cardConnected =
    enrollment.cardSerial != null &&
    cards.some((card) => card.serial === enrollment.cardSerial);
  const pinReason =
    access.props('yubi-pin', target).title ??
    (cards.length === 0
      ? 'No security key connected.'
      : !cardConnected
        ? 'This key’s card is not connected.'
        : undefined);
  // Every recovery action but "Resume enrollment" acts on a card whose
  // account already exists; that one acts on a card whose account does not,
  // so the two conditions are exact opposites rather than shades of one.
  const opReason =
    enrollment.state === 'complete'
      ? undefined
      : 'This enrollment is not complete.';
  const resumeReason =
    enrollment.state === 'pending'
      ? undefined
      : 'This enrollment is already complete.';
  return (
    <div
      className="settings-section"
      role="region"
      aria-labelledby="device-card-label"
    >
      <SectionLabel id="device-card-label">Card operations</SectionLabel>
      <Inset className="settings-inset middle wide">
        <InsetRow
          label="Connected"
          action={
            <>
              <Button
                size="sm"
                disabled={why !== undefined}
                title={why}
                onClick={onProvision}
              >
                Provision…
              </Button>
              <Button
                size="sm"
                disabled={pinReason !== undefined}
                title={pinReason}
                onClick={() => onYubiAction('pin-status', enrollment)}
              >
                PIN Status
              </Button>
            </>
          }
        >
          Read from the card in the port, not from the account.
        </InsetRow>
        {CARD_OP_GROUPS.map((group) => (
          <InsetRow
            key={group.label}
            label={group.label}
            action={
              <>
                {group.actions.map((action) => {
                  const reason =
                    access.props(YUBI_WORKFLOWS[action], target).title ??
                    (action === 'resume-enrollment' ? resumeReason : opReason);
                  return (
                    <Button
                      key={action}
                      size="sm"
                      disabled={reason !== undefined}
                      title={reason}
                      onClick={() => onYubiAction(action, enrollment)}
                    >
                      {CARD_OP_LABELS[action]}
                    </Button>
                  );
                })}
              </>
            }
          >
            {group.hint}
          </InsetRow>
        ))}
        <InsetRow
          label="PIN"
          action={
            <Button
              size="sm"
              onClick={() =>
                onNavigate({
                  kind: 'settings',
                  section: 'account',
                  profile: store.server,
                })
              }
            >
              Settings › Server
            </Button>
          }
        >
          Set in Settings › Server, along with the key’s other credentials.
        </InsetRow>
      </Inset>
    </div>
  );
}
