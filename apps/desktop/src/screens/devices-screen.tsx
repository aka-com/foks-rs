import { useDeviceMetadata } from '../device-cache';
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
 * FOKS calls a recovery phrase a backup phrase; this page calls the object a
 * paper key and keeps the enrollment name the agent stores.
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
import { enqueueProfileWork } from '../bridge';
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
  Notice,
  SectionLabel,
} from '../components';
import type {
  AccountStore,
  AgentSnapshot,
  StoreRef,
  DeviceLabel,
} from '../model';
import {
  accountStopped,
  accountStores,
  accountSubtitle,
  plural,
  serverName,
  shortId,
  usernameOf,
} from '../model';
import type { Location, NavigateOptions } from '../location';
import type { MutationFailureHandler } from '../mutation-recovery';
import { PageHeader } from '../shell/page-header';
import {
  AddDeviceSheet,
  EnrollSheet,
  PairSheet,
  PhraseSheet,
  ProvisionSheet,
  RecoverSheet,
  RemoveDeviceSheet,
  RevokeBackupSheet,
  RevokeSheet,
  YubiActionSheet,
} from './device-sheets';
import type { SimpleYubiAction } from './device-sheets';
import {
  deviceAt,
  deviceEntries,
  enrollmentForDevice,
  deviceIsCard,
} from './device-model';
import type { DeviceEntry } from './device-model';
import { UnavailableAccount } from './people-screen';

import { paperKeyResume } from './paper-key-resume';
import type { PaperKeyDraft } from './paper-key-resume';

type Sheet =
  | 'add'
  | 'pair'
  | 'phrase'
  | 'recover'
  | 'enrol'
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
  'recover-management': 'Restore management…',
  'recover-subkey': 'Restore signing key…',
  'resume-enrollment': 'Resume enrollment…',
  'resume-rotation': 'Resume rotation…',
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
  onRefresh: (message: string) => Promise<void>;
  onRefreshSnapshot: () => Promise<AgentSnapshot>;
  onError: (error: unknown) => void;
  onMutationError: MutationFailureHandler;
  onDeviceLabel?: (label: DeviceLabel | null) => void;
  /** Re-probe connected cards independently of cached enrollment metadata. */
  hardwareRefresh?: number;
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
  const stopped = selected
    ? accountStopped(snapshot, selected)
    : { stopped: true, reason: 'No account on this device' };
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
          ? 'enrol'
          : null,
    (value) => value === 'pair' && pairMode === 'accept',
  );
  const paperResume = paperKeyResume(bridge, selected?.id ?? '');
  const retainedPaperKey = useSyncExternalStore(
    paperResume.subscribe,
    paperResume.get,
  );
  const [resumedPaperKey, setResumedPaperKey] = useState<PaperKeyDraft | null>(
    null,
  );
  useEffect(() => () => paperResume.conceal(), [paperResume]);
  const [cards, setCards] = useState<{ serial: number }[]>([]);
  const [pendingYubi, setPendingYubi] = useState<SimpleYubiAction | null>(null);
  const [removing, setRemoving] = useState<AccountDevice | null>(null);
  const [revoking, setRevoking] = useState<BackupEnrollment | null>(null);
  // Which enrollment a per-row key action acts on. The row is the only thing
  // that says which one; there may be more than one.
  const [actingKey, setActingKey] = useState<YubiEnrollment | null>(null);

  const sheetOpen = useRef(sheet);
  sheetOpen.current = sheet;
  const {
    cache: deviceCache,
    lists,
    loading,
  } = useDeviceMetadata({
    bridge,
    profile: selected?.server,
    store: selected?.id,
    enabled: !stopped.stopped,
    recovery: { refresh: onRefreshSnapshot, allowed: () => !sheetOpen.current },
    onError,
  });
  const { devices, backups, yubi } = lists;

  const closeSheets = useCallback((): void => {
    paperResume.conceal();
    setResumedPaperKey(null);
    setSheet(null);
    setPendingYubi(null);
    setRemoving(null);
    setRevoking(null);
    setActingKey(null);
  }, [paperResume, setSheet]);

  // A phrase, a PIN or an unlock code must not stay on screen behind another
  // window.
  useEffect(() => {
    const conceal = (): void => closeSheets();
    const concealWhenHidden = (): void => {
      if (document.hidden) conceal();
    };
    window.addEventListener('blur', conceal);
    document.addEventListener('visibilitychange', concealWhenHidden);
    return () => {
      window.removeEventListener('blur', conceal);
      document.removeEventListener('visibilitychange', concealWhenHidden);
    };
  }, [closeSheets]);

  // Normalize the route to the active account's StoreRef. Because the
  // destination matches the current location, navigation guards are bypassed
  // and active dialogs remain open.
  useEffect(() => {
    if (location.store || !selected) return;
    onNavigate({ ...location, store: selected.id }, { force: true });
  }, [location, onNavigate, selected]);

  const selectedId = selected?.id;
  const profile = selected?.server;
  const accessStopped = stopped.stopped;

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
    let alive = true;
    setCards([]);
    if (profile && !accessStopped) {
      void enqueueProfileWork(bridge, profile, () =>
        bridge.listYubiCards(profile),
      )
        .then((cards) => {
          if (alive) setCards(cards);
        })
        .catch((error: unknown) => {
          if (alive) onError(error);
        });
    }
    return () => {
      alive = false;
    };
  }, [
    accessStopped,
    bridge,
    deviceCache,
    hardwareRefresh,
    onError,
    profile,
    selectedId,
  ]);

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
    await onRefresh(message);
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
          plural(backups.length, 'paper key'),
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
  // Every key on the account, sorted by type: computers, then paper keys,
  // then security key enrollments — the one list the page now shows.
  const entries = deviceEntries(lists);
  /** The row's trailing chip or button, ahead of the "Open" button every row
   * carries: which one depends on which of the three lists it came from. */
  const deviceRowAction = (entry: DeviceEntry): ReactNode => {
    const source = entry.source;
    if (source.kind === 'device') {
      const device = source.device;
      if (device.current) return <Chip tone="you">Current</Chip>;
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
        <Button
          size="sm"
          variant="danger"
          disabled={stopped.stopped}
          title={why}
          onClick={() => {
            setRevoking(backup);
            setSheet('revoke-backup');
          }}
        >
          Revoke…
        </Button>
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
  // are displayed; new accounts must be added via the Account tab.
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
                  onClick={() => onNavigate({ kind: 'people' })}
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
          stopped={stopped}
          cards={cards}
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
                disabled={stopped.stopped}
                title={why}
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
                          section: 'servers',
                          profile: selected.server,
                        })
                      }
                    >
                      Open the server…
                    </Button>
                  }
                >
                  Devices and keys are not listed while access is stopped.
                </Band>
              ) : null}

              <div
                className="settings-section"
                ref={listAnchor}
                tabIndex={-1}
                role="region"
                aria-labelledby="devices-all-label"
              >
                {/* Every key on the account is one list, sorted by type:
                    computers (and any device key on a card among them), then
                    paper keys, then security key enrollments — the order
                    `deviceEntries` already reads the three lists in. */}
                <SectionLabel id="devices-all-label">Devices</SectionLabel>
                <Inset className="settings-inset middle wide">
                  {loading ? (
                    <InsetRow label="Devices">
                      Loading devices, paper keys and security keys…
                    </InsetRow>
                  ) : entries.length ? (
                    entries.map((entry) => (
                      <InsetRow
                        key={entry.address}
                        className="devrow"
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
                        <span className="ic" aria-hidden="true">
                          <Icon name={entry.icon} />
                        </span>
                        <span className="t">
                          <b>{entry.name}</b>
                          <small>{kindLabel(entry.kind)}</small>
                          {entry.source.kind !== 'yubi' && entry.keyId ? (
                            <span className="kid" title={entry.keyId}>
                              {shortId(entry.keyId, 10)}
                            </span>
                          ) : null}
                        </span>
                      </InsetRow>
                    ))
                  ) : (
                    <InsetRow label="Devices">
                      {stopped.stopped
                        ? 'Not listed while access is stopped'
                        : failed
                          ? 'Devices and keys could not be read. Refresh to try again.'
                          : 'No devices, paper keys or security keys on this account.'}
                    </InsetRow>
                  )}
                </Inset>
              </div>
              {/* The two actions that make an account rather than act on a
                  device in the list above. */}
              <p className="fn">
                <Button
                  variant="plain"
                  size="sm"
                  className="lnk"
                  disabled={stopped.stopped}
                  title={why}
                  onClick={() => setSheet('recover')}
                >
                  Recover an account with a paper key…
                </Button>
                {' · '}
                <Button
                  variant="plain"
                  size="sm"
                  className="lnk"
                  disabled={stopped.stopped}
                  title={why}
                  onClick={() => setSheet('enrol')}
                >
                  Create an account on a YubiKey…
                </Button>
              </p>
            </div>
          </div>
        </>
      )}
      {sheet === 'add' && selected ? (
        <AddDeviceSheet
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
      {!sheet && retainedPaperKey?.hidden ? (
        <Band
          label="Paper key not saved"
          action={
            <>
              <Button
                onClick={() => {
                  setResumedPaperKey(paperResume.take());
                  setSheet('phrase');
                }}
              >
                Show the paper key again
              </Button>
              <Button onClick={paperResume.forget}>Dismiss paper key</Button>
            </>
          }
        >
          The phrase can be shown once more for two minutes after it was hidden.
        </Band>
      ) : null}
      {sheet === 'phrase' && selected ? (
        <PhraseSheet
          bridge={bridge}
          profile={selected.server}
          accountAlias={selected.account}
          seedAlias={resumedPaperKey?.alias}
          onPrepared={resumedPaperKey ? undefined : paperResume.prepare}
          onForget={paperResume.forget}
          seedPhrase={
            resumedPaperKey?.phrase ??
            (enteredScene === 'settings-phrase'
              ? bridge.firstRunFixture?.backupPhrase
              : undefined)
          }
          onClose={() => {
            setResumedPaperKey(null);
            setSheet(null);
          }}
          onDone={async () => {
            setResumedPaperKey(null);
            await applied('Paper key created.');
          }}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
      {sheet === 'recover' && selected ? (
        <RecoverSheet
          bridge={bridge}
          store={selected}
          onClose={() => setSheet(null)}
          onDone={async () => applied('Recovery submitted successfully.')}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
      {sheet === 'enrol' && selected ? (
        <EnrollSheet
          bridge={bridge}
          store={selected}
          card={cards[0]}
          onClose={() => setSheet(null)}
          onDone={async () => applied('YubiKey account created successfully.')}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
      {sheet === 'provision' && selected ? (
        <ProvisionSheet
          bridge={bridge}
          store={selected}
          cards={cards}
          onClose={() => setSheet(null)}
          onDone={async () => applied('YubiKey added to account successfully')}
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
            await applied('YubiKey revoked successfully');
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
            await applied(`Revoked paper key ${alias}`);
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
  stopped,
  cards,
  onBack,
  onNavigate,
  onCopy,
  onRemove,
  onRevokeBackup,
  onRevokeKey,
  onProvision,
  onYubiAction,
}: {
  snapshot: AgentSnapshot;
  store: AccountStore;
  /** The key the address names, once the lists have answered. */
  entry?: DeviceEntry;
  deviceEnrollment?: YubiEnrollment;
  loading: boolean;
  stopped: { stopped: boolean; reason: string };
  /** The cards in this Mac's ports right now. */
  cards: { serial: number }[];
  onBack: () => void;
  onNavigate: (location: Location) => void;
  onCopy: (text: string) => void;
  onRemove: (device: AccountDevice) => void;
  onRevokeBackup: (backup: BackupEnrollment) => void;
  onRevokeKey: (entry: YubiEnrollment) => void;
  onProvision: () => void;
  onYubiAction: (action: SimpleYubiAction, entry: YubiEnrollment) => void;
}): ReactNode {
  const why = stopped.stopped ? stopped.reason : undefined;
  if (!entry)
    return (
      <>
        <PageHeader
          ruled
          title="Devices"
          subtitle={accountSubtitle(snapshot, store)}
        />
        <div className="body">
          <div className="settings-main">
            <Notice
              severity={loading || stopped.stopped ? 'info' : 'crit'}
              title={
                stopped.stopped
                  ? 'Not listed while access is stopped'
                  : loading
                    ? 'Reading this account’s keys…'
                    : 'This key is not on this account'
              }
              actions={
                <Button variant="primary" onClick={onBack}>
                  Back to Devices
                </Button>
              }
            >
              <p>
                {stopped.stopped
                  ? stopped.reason
                  : loading
                    ? 'Loading devices, paper keys, and security keys…'
                    : 'This key is no longer associated with this account on this device.'}
              </p>
            </Notice>
          </div>
        </div>
      </>
    );

  const source = entry.source;
  const username = usernameOf(snapshot, store) ?? store.account;
  const enrollment = source.kind === 'yubi' ? source.entry : undefined;
  const revokeReason = stopped.stopped
    ? stopped.reason
    : enrollment && enrollment.state !== 'complete'
      ? 'This enrollment is not complete'
      : undefined;
  return (
    <>
      <PageHeader
        ruled
        title={entry.name}
        // An enrollment is the server's, not this account's, and the caption
        // says so rather than naming an account the agent did not answer for.
        subtitle={
          entry.scope === 'profile'
            ? `${kindLabel(entry.kind)} · on ${serverName(snapshot, store)}`
            : `${kindLabel(entry.kind)} · ${accountSubtitle(snapshot, store)}`
        }
        action={
          entry.current ? (
            <Chip tone="you">
              {entry.kind === 'Key on a card'
                ? 'Current key on a card'
                : 'This device'}
            </Chip>
          ) : enrollment ? (
            <Chip tone={enrollment.state === 'complete' ? 'ok' : 'warn'}>
              {enrollment.state === 'complete' ? 'Enrolled' : 'Incomplete'}
            </Chip>
          ) : undefined
        }
      />
      <div className="body">
        <div className="settings-main">
          <div
            className="settings-section"
            role="region"
            aria-labelledby="device-facts-label"
          >
            <SectionLabel id="device-facts-label">This key</SectionLabel>
            <Inset className="settings-inset middle wide">
              <InsetRow label="Kind">{kindLabel(entry.kind)}</InsetRow>
              {entry.role ? (
                <InsetRow label="Role on the account">{entry.role}</InsetRow>
              ) : null}
              {entry.scope === 'profile' ? (
                <InsetRow label="Server">
                  <b>{serverName(snapshot, store)}</b>
                  <small>This enrollment applies across the server.</small>
                </InsetRow>
              ) : (
                <InsetRow label="Account">
                  <span className="namechip">
                    <b>{username}</b> <Chip>{store.account}</Chip>
                  </span>
                  <small>on {serverName(snapshot, store)}</small>
                </InsetRow>
              )}
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
                  <small>
                    This is the key the agent signs this account’s operations
                    with here.
                  </small>
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
          <div
            className="settings-section"
            role="region"
            aria-labelledby="device-danger-label"
          >
            <SectionLabel className="danger-title" id="device-danger-label">
              Danger zone
            </SectionLabel>
            <Inset className="settings-inset middle wide danger-box">
              {/* A key on a card is the card's, whether or not this Mac is
                  authenticated with it now: it is not removed the way another
                  Mac is, so its kind decides this before its currency does. */}
              {source.kind === 'device' && entry.kind === 'Key on a card' ? (
                <InsetRow
                  className="dangerrow"
                  label="Revoked under its enrollment"
                  action={
                    <Button
                      size="sm"
                      onClick={() =>
                        onNavigate({
                          kind: 'devices',
                          section: 'keys',
                          store: store.id,
                          ...(deviceEnrollment
                            ? { device: `yubi:${deviceEnrollment.alias}` }
                            : {}),
                        })
                      }
                    >
                      {deviceEnrollment
                        ? `Open ${deviceEnrollment.alias}`
                        : 'Go to Devices'}
                    </Button>
                  }
                >
                  <small>
                    A key on a card is revoked under its enrollment, listed on
                    Devices.
                  </small>
                </InsetRow>
              ) : entry.current ? (
                <InsetRow
                  className="dangerrow"
                  label="This device cannot be removed here"
                  action={
                    <Button
                      size="sm"
                      onClick={() =>
                        onNavigate({ kind: 'settings', section: 'about' })
                      }
                    >
                      Open Settings
                    </Button>
                  }
                >
                  <small>
                    Remove this device from another device. Reset this device,
                    in Settings, erases everything it holds for every server.
                  </small>
                </InsetRow>
              ) : source.kind === 'device' ? (
                <InsetRow
                  className="dangerrow"
                  label="Remove this device"
                  action={
                    <Button
                      size="sm"
                      variant="danger"
                      disabled={stopped.stopped}
                      title={why}
                      onClick={() => onRemove(source.device)}
                    >
                      Remove this device…
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
                  label="Revoke this paper key"
                  action={
                    <Button
                      size="sm"
                      variant="danger"
                      disabled={stopped.stopped}
                      title={why}
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
        </div>
      </div>
    </>
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
  stopped,
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
  const why = stopped.stopped ? stopped.reason : undefined;
  const cardConnected =
    enrollment.cardSerial != null &&
    cards.some((card) => card.serial === enrollment.cardSerial);
  const pinReason = stopped.stopped
    ? stopped.reason
    : cards.length === 0
      ? 'No security key connected.'
      : !cardConnected
        ? 'This key’s card is not connected.'
        : undefined;
  // Every recovery action but "Resume enrollment" acts on a card whose
  // account already exists; that one acts on a card whose account does not,
  // so the two conditions are exact opposites rather than shades of one.
  const opReason = stopped.stopped
    ? stopped.reason
    : enrollment.state === 'complete'
      ? undefined
      : 'This enrollment is not complete.';
  const resumeReason = stopped.stopped
    ? stopped.reason
    : enrollment.state === 'pending'
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
                disabled={stopped.stopped}
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
                PIN status
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
                    action === 'resume-enrollment' ? resumeReason : opReason;
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
                  section: 'servers',
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
