/**
 * The Devices tab: one page per account, with no sub-navigation.
 *
 * Recovery devices and Security keys were two panes two levels down; they are
 * three sections of one page here. Every row carries only what the agent
 * returns — a name, a role, a key id, whether it is this Mac — and each kind's
 * destructive action carries the rest in its typed confirmation. The three
 * ways to add a key are one chooser.
 *
 * FOKS calls a recovery phrase a backup phrase; this page calls the object a
 * paper key and keeps the enrollment name the agent stores.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { useToast } from '/kit/toasts';
import { normalizeCommandError } from '../bridge';
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
} from '../components';
import type { AccountStore, AgentSnapshot, StoreRef } from '../model';
import {
  accountStopped,
  accountStores,
  accountSubtitle,
  plural,
  serverName,
  shortId,
  usernameOf,
} from '../model';
import type { Location } from '../location';
import type { MutationFailureHandler } from '../mutation-recovery';
import { PageHeader } from '../shell/page-header';
import { AccountSwitcher } from './account-switcher';
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
  YUBI_ACTION_LABELS,
  YubiActionSheet,
} from './device-sheets';
import type { SimpleYubiAction } from './device-sheets';
import {
  deviceAt,
  deviceIsCard,
  deviceName,
  readAccountAndProfileKeys,
  roleLabel,
} from './device-model';
import type { DeviceEntry, DeviceLists } from './device-model';
import { UnavailableAccount } from './people-screen';

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
 * The card operations that are recovery paths for a key already in trouble.
 * Each menu entry and the sheet it opens read the same label.
 */
const RECOVERY_ACTIONS: readonly SimpleYubiAction[] = [
  'sync',
  'set-passphrase',
  'change-passphrase',
  'verify-passphrase',
  'recover-management',
  'recover-subkey',
  'resume-enrollment',
  'resume-rotation',
  'rotate',
];

export interface DevicesScreenProps {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  location: Extract<Location, { kind: 'devices' }>;
  /** The named fixture scene this tab was entered at, captured once. */
  scene: string;
  onNavigate: (location: Location) => void;
  onRefresh: (message: string) => Promise<void>;
  onRefreshSnapshot: () => Promise<AgentSnapshot>;
  onError: (error: unknown) => void;
  onMutationError: MutationFailureHandler;
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
    : { stopped: true, reason: 'No account on this Mac' };
  const [sheet, setSheet] = useState<Sheet>(() =>
    enteredScene === 'settings-phrase'
      ? 'phrase'
      : enteredScene === 'settings-enrol'
        ? 'enrol'
        : null,
  );
  const [devices, setDevices] = useState<AccountDevice[]>([]);
  const [backups, setBackups] = useState<BackupEnrollment[]>([]);
  const [yubi, setYubi] = useState<YubiEnrollment[]>([]);
  const [cards, setCards] = useState<{ serial: number }[]>([]);
  // The first paint has read nothing yet, so the page loads until the four
  // calls answer rather than reporting three zeroes.
  const [loading, setLoading] = useState(true);
  const [pendingYubi, setPendingYubi] = useState<SimpleYubiAction | null>(null);
  const [pairMode, setPairMode] = useState<'offer' | 'accept'>('offer');
  const [removing, setRemoving] = useState<AccountDevice | null>(null);
  const [revoking, setRevoking] = useState<BackupEnrollment | null>(null);
  // Which enrollment a per-row key action acts on. The row is the only thing
  // that says which one; there may be more than one.
  const [actingKey, setActingKey] = useState<YubiEnrollment | null>(null);

  const sheetOpen = useRef(sheet);
  sheetOpen.current = sheet;
  const catalogRecovery = useRef<Promise<AgentSnapshot> | null>(null);
  const recovered = useRef(new Set<string>());
  // The shell's callback is held in a ref, so the four reads below are driven
  // by the account they are about and not by the identity of a function the
  // caller may rebuild on every render.
  const refreshSnapshot = useRef(onRefreshSnapshot);
  refreshSnapshot.current = onRefreshSnapshot;
  // Deduplicate concurrent catalog recovery requests.
  const recoverCatalog = useCallback((): Promise<AgentSnapshot> => {
    if (!catalogRecovery.current) {
      const pending = refreshSnapshot.current().finally(() => {
        if (catalogRecovery.current === pending) catalogRecovery.current = null;
      });
      catalogRecovery.current = pending;
    }
    return catalogRecovery.current;
  }, []);

  const closeSheets = useCallback((): void => {
    setSheet(null);
    setPendingYubi(null);
    setRemoving(null);
    setRevoking(null);
    setActingKey(null);
  }, []);

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

  // Canonicalize the default route to the first account's exact StoreRef.
  useEffect(() => {
    if (location.store || !selected) return;
    onNavigate({ ...location, store: selected.id });
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

  // Four separate calls with four separate failure modes. They are read
  // together because the page lists them together, and a catalog the agent
  // replaced mid-read is recovered once and retried once.
  useEffect(() => {
    let alive = true;
    setDevices([]);
    setBackups([]);
    setYubi([]);
    setCards([]);
    if (!selectedId || !profile || accessStopped) {
      setLoading(false);
      return;
    }
    setLoading(true);
    const load = (): Promise<DeviceLists> =>
      readAccountAndProfileKeys(bridge, profile, selectedId);
    void (async () => {
      try {
        let result: DeviceLists;
        try {
          result = await load();
        } catch (error) {
          if (!alive) return;
          const retryable =
            !sheetOpen.current &&
            normalizeCommandError(error).code === 'catalog-required';
          if (!retryable) throw error;
          const already = recovered.current.has(selectedId);
          if (already && !catalogRecovery.current) throw error;
          recovered.current.add(selectedId);
          await recoverCatalog();
          if (!alive) return;
          result = await load();
        }
        if (!alive) return;
        recovered.current.delete(selectedId);
        setDevices(result.devices);
        setBackups(result.backups);
        setCards(result.cards);
        setYubi(result.yubi);
        setLoading(false);
      } catch (error) {
        if (alive) {
          onError(error);
          setLoading(false);
        }
      }
    })();
    return () => {
      alive = false;
    };
  }, [accessStopped, bridge, onError, profile, recoverCatalog, selectedId]);

  // A `section=` address — the Devices addresses written before the page was
  // one — lands on the section it names.
  const anchors = {
    macs: useRef<HTMLDivElement>(null),
    keys: useRef<HTMLDivElement>(null),
  };
  const requestedSection = location.section;
  useEffect(() => {
    if (!requestedSection) return;
    // If an initial scene immediately opens a dialog (e.g., `settings-phrase`),
    // preserve focus within the dialog rather than shifting focus to the
    // background section anchor until the dialog is closed.
    if (sheet) return;
    const anchor = anchors[requestedSection].current;
    if (!anchor) return;
    if (typeof anchor.scrollIntoView === 'function')
      anchor.scrollIntoView({ block: 'start' });
    anchor.focus({ preventScroll: true });
    // The anchors are stable refs; the address and the open sheet are what
    // move the page.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [requestedSection, sheet]);

  // `onRefresh` reloads the catalog and says what happened; the page does not
  // repeat the sentence.
  const applied = async (message: string): Promise<void> => {
    closeSheets();
    await onRefresh(message);
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
          plural(devices.length - cardKeys, 'Mac'),
          ...(cardKeys
            ? [plural(cardKeys, 'key on a card', 'keys on cards')]
            : []),
          plural(backups.length, 'paper key'),
          // `list_yubi_accounts` answers with enrollments, not with keys: one
          // count per object, under the name that object carries everywhere.
          plural(yubi.length, 'enrollment'),
        ].join(' · ');
  const complete = yubi.find((entry) => entry.state === 'complete');
  const pending = yubi.find((entry) => entry.state === 'pending');
  const why = stopped.stopped ? stopped.reason : undefined;
  const openYubi = (action: SimpleYubiAction, entry?: YubiEnrollment): void => {
    setActingKey(entry ?? null);
    setPendingYubi(action);
    setSheet('yubi');
  };
  const yubiReason = (action: SimpleYubiAction): string | undefined => {
    if (stopped.stopped) return stopped.reason;
    if (loading) return 'Loading this account…';
    if (action === 'resume-enrollment')
      return pending ? undefined : 'No pending enrollment found';
    // The card's PIN is read from the card in the port, not from an
    // enrollment, so a connected card is most of what this one needs — but
    // the command still names an enrollment, and this account may have none.
    if (action === 'pin-status')
      return cards.length
        ? (complete ?? pending)
          ? undefined
          : 'No enrollment on the connected card'
        : 'No security key is connected';
    return complete ? undefined : 'No complete enrollment found';
  };
  // The key a `device=` address names, once the four lists have answered.
  const detail = location.device
    ? deviceAt({ devices, backups, yubi, cards }, location.device)
    : undefined;

  // An address naming an account this Mac no longer holds is reported as
  // exactly that, and the notice is the only list of the accounts it does
  // hold: a switcher beside it would offer the same accounts twice.
  // "No account configured" would be a different, false statement.
  if (!selected && requested)
    return (
      <>
        <PageHeader title="Devices" subtitle="" />
        <div className="body">
          <div className="settings-main">
            <UnavailableAccount
              stores={stores}
              snapshot={snapshot}
              onSelect={(store) => onNavigate(listAt(store.id))}
              onRefresh={() => void onRefresh('Accounts refreshed')}
            />
          </div>
        </div>
      </>
    );

  // Devices is per account, so a Mac with none has nothing to list; the
  // account is made on Accounts, where accounts are.
  if (!selected)
    return (
      <>
        <PageHeader title="Devices" subtitle="" />
        <div className="body">
          <div className="settings-main">
            <Notice
              title="No account configured"
              actions={
                <Button
                  variant="primary"
                  onClick={() => onNavigate({ kind: 'people' })}
                >
                  Open Accounts
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
          loading={loading}
          stopped={stopped}
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
        />
      ) : (
        <>
          <PageHeader
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
                Add a device or paper key
              </Button>
            }
          />
          <div className="body">
            <div className="settings-main">
              <SectionLabel id="devices-accounts-label">
                Accounts on this Mac
              </SectionLabel>
              <AccountSwitcher
                snapshot={snapshot}
                stores={stores}
                selected={selected}
                labelledBy="devices-accounts-label"
                onSwitch={(store) => onNavigate(listAt(store.id))}
              />
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
                ref={anchors.macs}
                tabIndex={-1}
                role="region"
                aria-labelledby="devices-macs-label"
              >
                {/* The region holds this account's authenticated devices, and
                    a device key on a card is one of them; naming it for the
                    Macs alone would name half of what it encloses. */}
                <SectionLabel id="devices-macs-label">
                  Macs and device keys
                </SectionLabel>
                <Inset className="settings-inset middle wide">
                  {loading ? (
                    <InsetRow label="None">Loading devices…</InsetRow>
                  ) : devices.length ? (
                    devices.map((device) => (
                      <InsetRow
                        key={device.id}
                        className="devrow"
                        action={
                          <>
                            {device.current ? (
                              <Chip tone="you">
                                {deviceIsCard(device)
                                  ? 'Current key on a card'
                                  : 'This device'}
                              </Chip>
                            ) : !deviceIsCard(device) ? (
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
                            ) : (
                              <Chip>Key on a card</Chip>
                            )}
                            <OpenDevice
                              name={deviceName(device)}
                              onOpen={() => openDevice(device.id)}
                            />
                          </>
                        }
                      >
                        <span className="ic" aria-hidden="true">
                          <Icon
                            name={deviceIsCard(device) ? 'key' : 'laptop'}
                          />
                        </span>
                        <span className="t">
                          <b>{deviceName(device)}</b>
                          <small>
                            {deviceIsCard(device)
                              ? 'Key on a card'
                              : 'Computer'}{' '}
                            · <span>{roleLabel(device.role)}</span>
                          </small>
                          <span className="kid" title={device.id}>
                            {shortId(device.id, 10)}
                          </span>
                        </span>
                      </InsetRow>
                    ))
                  ) : (
                    <InsetRow label="None">
                      {stopped.stopped
                        ? 'Not listed while access is stopped'
                        : 'No Macs or keys on cards are authenticated on this account.'}
                    </InsetRow>
                  )}
                </Inset>
              </div>
              {/* Paper keys are their own section, so they are their own region:
              a region named "Your Macs" that held them would name half of
              what it encloses. */}
              <div
                className="settings-section"
                role="region"
                aria-labelledby="devices-paper-label"
              >
                <SectionLabel
                  id="devices-paper-label"
                  action={
                    <Button
                      size="sm"
                      disabled={stopped.stopped}
                      title={why}
                      onClick={() => setSheet('phrase')}
                    >
                      Add a paper key…
                    </Button>
                  }
                >
                  Paper keys
                </SectionLabel>
                <Inset className="settings-inset middle wide">
                  {loading ? (
                    <InsetRow label="None">Loading paper keys…</InsetRow>
                  ) : backups.length ? (
                    backups.map((backup) => (
                      <InsetRow
                        key={backup.backupId}
                        className="devrow"
                        action={
                          <>
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
                            <OpenDevice
                              name={backup.backupAlias}
                              onOpen={() => openDevice(backup.backupId)}
                            />
                          </>
                        }
                      >
                        <span className="ic" aria-hidden="true">
                          <Icon name="file" />
                        </span>
                        <span className="t">
                          <b>{backup.backupAlias}</b>
                          <span className="kid" title={backup.backupId}>
                            {shortId(backup.backupId, 10)}
                          </span>
                        </span>
                      </InsetRow>
                    ))
                  ) : (
                    <InsetRow label="None">
                      {stopped.stopped
                        ? 'Not listed while access is stopped'
                        : 'No paper keys stored on this Mac for this account.'}
                    </InsetRow>
                  )}
                  <InsetRow
                    label="Recover an account"
                    action={
                      <Button
                        size="sm"
                        disabled={stopped.stopped}
                        title={why}
                        onClick={() => setSheet('recover')}
                      >
                        Recover…
                      </Button>
                    }
                  >
                    Use a paper key to recover an existing account on this Mac.
                  </InsetRow>
                </Inset>
                <p className="fn">
                  Only paper keys stored on this Mac are listed.
                </p>
              </div>
              <div
                className="settings-section"
                ref={anchors.keys}
                tabIndex={-1}
                role="region"
                aria-labelledby="devices-keys-label"
              >
                <SectionLabel
                  id="devices-keys-label"
                  action={
                    <MenuButton
                      size="sm"
                      label="More…"
                      menuLabel="Security key operations"
                      align="end"
                    >
                      {(close) => (
                        <>
                          <MenuItem
                            icon="plus"
                            reason={
                              stopped.stopped ? stopped.reason : undefined
                            }
                            onClick={() => {
                              close();
                              setSheet('enrol');
                            }}
                          >
                            Create an account on a YubiKey…
                          </MenuItem>
                          <hr />
                          {RECOVERY_ACTIONS.map((action) => (
                            <MenuItem
                              key={action}
                              reason={yubiReason(action)}
                              onClick={() => {
                                close();
                                openYubi(action);
                              }}
                            >
                              {YUBI_ACTION_LABELS[action]}…
                            </MenuItem>
                          ))}
                        </>
                      )}
                    </MenuButton>
                  }
                >
                  Security key enrollments
                </SectionLabel>
                <Inset className="settings-inset middle wide">
                  {loading ? (
                    <InsetRow label="None">Loading keys…</InsetRow>
                  ) : yubi.length ? (
                    yubi.map((entry) => (
                      <InsetRow
                        key={entry.alias}
                        className="devrow"
                        action={
                          <>
                            <Chip
                              tone={entry.state === 'complete' ? 'ok' : 'warn'}
                            >
                              {entry.state === 'complete'
                                ? 'Enrolled'
                                : 'Incomplete'}
                            </Chip>
                            <Button
                              size="sm"
                              variant="danger"
                              disabled={
                                stopped.stopped || entry.state !== 'complete'
                              }
                              title={
                                stopped.stopped
                                  ? stopped.reason
                                  : entry.state === 'complete'
                                    ? undefined
                                    : 'This enrollment is not complete'
                              }
                              onClick={() => {
                                setActingKey(entry);
                                setSheet('revoke');
                              }}
                            >
                              Revoke…
                            </Button>
                            <OpenDevice
                              name={entry.alias}
                              onOpen={() => openDevice(`yubi:${entry.alias}`)}
                            />
                          </>
                        }
                      >
                        <span className="ic" aria-hidden="true">
                          <Icon name="key" />
                        </span>
                        <span className="t">
                          <b>{entry.alias}</b>
                          {/* The agent reports no serial for an enrollment, so the
                          row says so rather than leaving the reader to match
                          it against the card below. */}
                          <small>
                            Serial number unavailable for an enrollment
                          </small>
                        </span>
                      </InsetRow>
                    ))
                  ) : (
                    <InsetRow label="None">
                      {stopped.stopped
                        ? 'Not listed while access is stopped'
                        : 'No YubiKeys enrolled.'}
                    </InsetRow>
                  )}
                  {cards.length ? (
                    cards.map((card) => (
                      <InsetRow
                        key={card.serial}
                        label="Connected now"
                        action={
                          <>
                            <Button
                              size="sm"
                              disabled={stopped.stopped}
                              title={why}
                              onClick={() => setSheet('provision')}
                            >
                              Provision…
                            </Button>
                            <Button
                              size="sm"
                              disabled={yubiReason('pin-status') !== undefined}
                              title={yubiReason('pin-status')}
                              onClick={() =>
                                openYubi('pin-status', complete ?? pending)
                              }
                            >
                              PIN status
                            </Button>
                          </>
                        }
                      >
                        <b>YubiKey {card.serial}</b>{' '}
                        <Chip tone="ok">Connected</Chip>
                        <small>
                          Read from the card in the port, not from the account.
                        </small>
                      </InsetRow>
                    ))
                  ) : (
                    <InsetRow label="Connected now">
                      {stopped.stopped
                        ? 'Not read while access is stopped'
                        : 'No security key is connected.'}
                    </InsetRow>
                  )}
                  <InsetRow
                    label="Card PIN"
                    action={
                      <Button
                        size="sm"
                        onClick={() =>
                          onNavigate({
                            kind: 'settings',
                            section: 'credentials',
                            // The address the page carries, so a stale reference
                            // travels and is reported there rather than dropped.
                            store: location.store ?? selected.id,
                          })
                        }
                      >
                        Settings › Account
                      </Button>
                    }
                  >
                    Changed with the other credentials you type.
                  </InsetRow>
                </Inset>
                <p className="fn">
                  Enrollments are listed for {serverName(snapshot, selected)},
                  across all accounts on that server.
                </p>
              </div>
            </div>
          </div>
        </>
      )}
      {sheet === 'add' && selected ? (
        <AddDeviceSheet
          subtitle={accountSubtitle(snapshot, selected)}
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
          subtitle={accountSubtitle(snapshot, selected)}
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
          username={
            snapshot.accounts.find((entry) => entry.store === selected.id)
              ?.username ?? selected.account
          }
          server={serverName(snapshot, selected)}
          seedPhrase={
            enteredScene === 'settings-phrase'
              ? bridge.firstRunFixture?.backupPhrase
              : undefined
          }
          onClose={() => setSheet(null)}
          onDone={async () => applied('Paper key created.')}
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
          // A row's action names its own enrollment; the section's menu acts
          // on the one enrollment the operation applies to.
          alias={
            (
              actingKey ??
              (pendingYubi === 'resume-enrollment' ? pending : complete)
            )?.alias ?? ''
          }
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
      {sheet === 'revoke' && selected && (actingKey ?? complete) ? (
        <RevokeSheet
          bridge={bridge}
          store={selected}
          alias={(actingKey ?? complete)?.alias ?? ''}
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
  loading,
  stopped,
  onBack,
  onNavigate,
  onCopy,
  onRemove,
  onRevokeBackup,
  onRevokeKey,
}: {
  snapshot: AgentSnapshot;
  store: AccountStore;
  /** The key the address names, once the lists have answered. */
  entry?: DeviceEntry;
  loading: boolean;
  stopped: { stopped: boolean; reason: string };
  onBack: () => void;
  onNavigate: (location: Location) => void;
  onCopy: (text: string) => void;
  onRemove: (device: AccountDevice) => void;
  onRevokeBackup: (backup: BackupEnrollment) => void;
  onRevokeKey: (entry: YubiEnrollment) => void;
}): ReactNode {
  const why = stopped.stopped ? stopped.reason : undefined;
  if (!entry)
    return (
      <>
        <PageHeader
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
                    : 'The address names a key this account no longer holds on this Mac.'}
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
        title={entry.name}
        // An enrollment is the server's, not this account's, and the caption
        // says so rather than naming an account the agent did not answer for.
        subtitle={
          entry.scope === 'profile'
            ? `${entry.kind} · on ${serverName(snapshot, store)}`
            : `${entry.kind} · ${accountSubtitle(snapshot, store)}`
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
              <InsetRow label="Kind">{entry.kind}</InsetRow>
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
                      The agent reports no key id for an enrollment, and no
                      serial for the card it was made on.
                    </small>
                  </>
                )}
              </InsetRow>
              {entry.current ? (
                <InsetRow label="Status">
                  Authenticated on this Mac now
                  <small>
                    This is the key the agent signs this account’s operations
                    with here.
                  </small>
                </InsetRow>
              ) : null}
            </Inset>
          </div>
          {source.kind === 'yubi' ? (
            <div
              className="settings-section"
              role="region"
              aria-labelledby="device-card-label"
            >
              <SectionLabel id="device-card-label">Card</SectionLabel>
              <Inset className="settings-inset middle wide">
                <InsetRow
                  label="Card PIN"
                  action={
                    <Button
                      size="sm"
                      onClick={() =>
                        onNavigate({
                          kind: 'settings',
                          section: 'credentials',
                          store: store.id,
                        })
                      }
                    >
                      Settings › Account
                    </Button>
                  }
                >
                  Changed with the other credentials you type.
                </InsetRow>
              </Inset>
            </div>
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
                        })
                      }
                    >
                      Go to Security key enrollments
                    </Button>
                  }
                >
                  <small>
                    A key on a card is revoked under Security key enrollments,
                    where the enrollment it was made under is listed.
                  </small>
                </InsetRow>
              ) : entry.current ? (
                <InsetRow
                  className="dangerrow"
                  label="This Mac cannot remove itself"
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
                    A device is removed from another device. Reset this Mac, in
                    Settings, erases what this Mac holds for every server.
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
                    {entry.name} loses access to every vault, group and chat on
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
