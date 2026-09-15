/**
 * The Settings tab: one scrolling page, with no sub-navigation.
 *
 * What is left once Accounts holds the accounts, Devices holds the keys and
 * Teams holds the groups: the servers this Mac talks to, the credentials you
 * type, what this application and its agent are, and the one reset that acts
 * on this Mac. A `section=` address scrolls to and focuses its section, and
 * `profile=` opens that server's page.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import type { ReactNode, RefObject } from 'react';
import { useToast } from '/kit/toasts';
import { enqueueProfileWork, normalizeCommandError } from '../bridge';
import type { AppInfo, Bridge, ResetPreview, YubiEnrollment } from '../bridge';
import {
  Band,
  Button,
  Chip,
  Icon,
  Inset,
  InsetRow,
  SectionLabel,
  SheetDialog,
} from '../components';
import type { Location, SettingsSection } from '../location';
import {
  accountStopped,
  accountStores,
  accountSubtitle,
  plural,
  serverDisplayName,
  serverName,
  usernameOf,
} from '../model';
import type { AccountStore, AgentSnapshot } from '../model';
import { PageHeader } from '../shell/page-header';
import type { MutationFailureHandler } from '../mutation-recovery';
import { agentLifecycleLabel, type AgentLifecycle } from '../agent-lifecycle';
import { NotificationSettings } from '../chat/notification-provider';
import { ServersSection } from './servers-screen';
import { AccountMark, AccountSwitcher } from './account-switcher';
import { UnavailableAccount } from './people-screen';
import { PassphraseSheet, YubiActionSheet } from './device-sheets';
import type { PassphraseMode, SimpleYubiAction } from './device-sheets';

export interface SettingsScreenProps {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  location: Extract<Location, { kind: 'settings' }>;
  scene: string;
  onNavigate: (location: Location) => void;
  onRefresh: (message: string) => Promise<void>;
  onRefreshSnapshot: () => Promise<AgentSnapshot>;
  onError: (error: unknown) => void;
  onMutationError: MutationFailureHandler;
  onLock: () => Promise<boolean>;
  agentLifecycle: AgentLifecycle;
  onRetryAgent: () => Promise<void>;
}

/** A sheet this page owns. Add a server and the per-server reset are the
 *  Servers section's own. */
type Sheet = 'passphrase' | 'yubi' | 'reset-mac' | null;

export function SettingsScreen({
  snapshot,
  bridge,
  location,
  scene,
  onNavigate,
  onRefresh,
  onRefreshSnapshot,
  onError,
  onMutationError,
  onLock,
  agentLifecycle,
  onRetryAgent,
}: SettingsScreenProps): ReactNode {
  const [enteredScene] = useState(scene);
  const toasts = useToast();
  const stores = accountStores(snapshot);
  const addressed = location.store
    ? stores.find((store) => store.id === location.store)
    : stores[0];
  // An address naming an account this Mac no longer holds: the card rows act
  // on nothing, and saying "No security key is enrolled" would be a claim
  // about an account that is not here.
  const unavailable = location.store !== undefined && addressed === undefined;
  const [sheet, setSheet] = useState<Sheet>(null);
  const [passphrase, setPassphrase] = useState<{
    store: AccountStore;
    mode: PassphraseMode;
  } | null>(null);
  const [yubiAction, setYubiAction] = useState<SimpleYubiAction>('change-pin');
  const [yubi, setYubi] = useState<YubiEnrollment[]>([]);
  const [appInfo, setAppInfo] = useState<AppInfo | null>(null);

  // A passphrase, a PIN or an unlock code must not stay on screen behind
  // another window, and a reset preview token is invalidated with it.
  useEffect(() => {
    const conceal = (): void => {
      setSheet(null);
      setPassphrase(null);
    };
    const concealWhenHidden = (): void => {
      if (document.hidden) conceal();
    };
    window.addEventListener('blur', conceal);
    document.addEventListener('visibilitychange', concealWhenHidden);
    return () => {
      window.removeEventListener('blur', conceal);
      document.removeEventListener('visibilitychange', concealWhenHidden);
    };
  }, []);

  useEffect(() => {
    let alive = true;
    void bridge
      .appInfo()
      .then((info) => {
        if (alive) setAppInfo(info);
      })
      .catch(onError);
    return () => {
      alive = false;
    };
  }, [bridge, onError]);

  // The card rows act on the addressed account's enrolled key; the key itself
  // is listed on Devices.
  const profile = addressed?.server;
  const keysStopped = addressed
    ? accountStopped(snapshot, addressed).stopped
    : true;
  useEffect(() => {
    let alive = true;
    setYubi([]);
    if (!profile || keysStopped) return;
    void enqueueProfileWork(bridge, profile, () =>
      bridge.listYubiAccounts(profile),
    )
      .then((entries) => {
        if (alive) setYubi(entries);
      })
      .catch((error: unknown) => {
        if (alive && normalizeCommandError(error).code !== 'catalog-required')
          onError(error);
      });
    return () => {
      alive = false;
    };
  }, [bridge, keysStopped, onError, profile]);

  const enrolled = yubi.find((entry) => entry.state === 'complete');
  const openYubi = (action: SimpleYubiAction): void => {
    setYubiAction(action);
    setSheet('yubi');
  };
  const cardReason = keysStopped
    ? 'Account access is stopped'
    : enrolled
      ? undefined
      : 'No complete enrollment found';

  // A `section=` address scrolls to its section and puts the keyboard there.
  const anchors = {
    servers: useRef<HTMLDivElement>(null),
    credentials: useRef<HTMLDivElement>(null),
    about: useRef<HTMLDivElement>(null),
    notifications: useRef<HTMLDivElement>(null),
  } satisfies Record<SettingsSection, unknown>;
  const requestedSection = location.section;
  const openProfile = location.profile;
  useEffect(() => {
    if (!requestedSection || openProfile) return;
    const anchor = anchors[requestedSection]?.current;
    if (!anchor) return;
    if (typeof anchor.scrollIntoView === 'function')
      anchor.scrollIntoView({ block: 'start' });
    anchor.focus({ preventScroll: true });
    // The anchors are stable refs; the address is what moves the page.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [requestedSection, openProfile]);

  const serversSection = (
    <ServersSection
      snapshot={snapshot}
      bridge={bridge}
      profile={location.profile}
      scene={enteredScene}
      onNavigate={onNavigate}
      onRefresh={onRefresh}
      onError={onError}
      onMutationError={onMutationError}
    />
  );

  // A server's own page is a page, not a section of this one.
  if (location.profile)
    return (
      <>
        <PageHeader title="Settings" subtitle="Servers" />
        <div className="body">
          <div className="settings-main">{serversSection}</div>
        </div>
      </>
    );

  return (
    <>
      <PageHeader
        title="Settings"
        subtitle="Servers, account credentials and this Mac"
      />
      <div className="body">
        <div className="settings-main">
          <div
            className="settings-section"
            ref={anchors.servers}
            tabIndex={-1}
            role="region"
            // This section's first label names a group of servers ("Needs
            // attention", "Ready"), not the section, so it is named here.
            aria-label="Servers"
          >
            {serversSection}
          </div>
          <div
            className="settings-section"
            ref={anchors.credentials}
            tabIndex={-1}
            role="region"
            aria-labelledby="settings-account-label"
          >
            <SectionLabel id="settings-account-label">Account</SectionLabel>
            <Inset className="settings-inset middle wide">
              {stores.length ? (
                stores.map((store) => {
                  const stopped = accountStopped(snapshot, store);
                  return (
                    <InsetRow
                      key={store.id}
                      className="devrow"
                      action={
                        <>
                          {(['set', 'change', 'verify'] as const).map(
                            (mode) => (
                              <Button
                                key={mode}
                                size="sm"
                                disabled={stopped.stopped}
                                title={
                                  stopped.stopped ? stopped.reason : undefined
                                }
                                onClick={() => {
                                  setPassphrase({ store, mode });
                                  setSheet('passphrase');
                                }}
                              >
                                {mode === 'set'
                                  ? 'Set…'
                                  : mode === 'change'
                                    ? 'Change…'
                                    : 'Verify…'}
                              </Button>
                            ),
                          )}
                        </>
                      }
                    >
                      <AccountMark
                        name={usernameOf(snapshot, store) ?? store.account}
                      />
                      <span className="t">
                        <b>{usernameOf(snapshot, store) ?? store.account}</b>
                        <small>
                          {store.account} · {serverName(snapshot, store)}
                        </small>
                        {/* Why the three actions are off, on the row and not
                            only in each button's title. */}
                        {stopped.stopped ? (
                          <small className="why">{stopped.reason}</small>
                        ) : null}
                      </span>
                    </InsetRow>
                  );
                })
              ) : (
                <InsetRow label="None">No accounts on this Mac.</InsetRow>
              )}
            </Inset>
            <SectionLabel id="settings-card-label">
              {addressed
                ? `Security key credentials · ${accountSubtitle(snapshot, addressed)}`
                : 'Security key credentials'}
            </SectionLabel>
            {/* The label names one account, so the reader is given the way to
                choose another rather than a fact about an account they did
                not pick. */}
            {unavailable ? null : (
              <AccountSwitcher
                snapshot={snapshot}
                stores={stores}
                selected={addressed}
                labelledBy="settings-card-label"
                onSwitch={(store) =>
                  onNavigate({ ...location, store: store.id })
                }
              />
            )}
            {unavailable ? (
              <UnavailableAccount
                stores={stores}
                snapshot={snapshot}
                onSelect={(store) =>
                  onNavigate({ ...location, store: store.id })
                }
                onRefresh={() => void onRefresh('Accounts refreshed')}
              />
            ) : (
              <Inset className="settings-inset middle wide">
                <InsetRow
                  label="Enrolled key"
                  action={
                    <Button
                      size="sm"
                      onClick={() =>
                        onNavigate({
                          kind: 'devices',
                          // The address this page carries, so a stale reference
                          // travels and is reported there rather than dropped.
                          ...((location.store ?? addressed?.id)
                            ? { store: location.store ?? addressed?.id }
                            : {}),
                        })
                      }
                    >
                      Open Devices
                    </Button>
                  }
                >
                  {enrolled ? (
                    <>
                      <b>{enrolled.alias}</b> <Chip tone="ok">Enrolled</Chip>
                    </>
                  ) : (
                    'No security key is enrolled on this account.'
                  )}
                  <small>
                    The key itself, its enrollment and PIN status are on the
                    Devices page.
                  </small>
                </InsetRow>
                <InsetRow
                  label="Card PIN"
                  action={
                    <Button
                      size="sm"
                      disabled={cardReason !== undefined}
                      title={cardReason}
                      onClick={() => openYubi('change-pin')}
                    >
                      Change PIN…
                    </Button>
                  }
                >
                  Enter the current PIN and a new PIN.
                </InsetRow>
                <InsetRow
                  label="Unlock code"
                  action={
                    <>
                      <Button
                        size="sm"
                        disabled={cardReason !== undefined}
                        title={cardReason}
                        onClick={() => openYubi('unblock')}
                      >
                        Unblock PIN…
                      </Button>
                      <Button
                        size="sm"
                        disabled={cardReason !== undefined}
                        title={cardReason}
                        onClick={() => openYubi('change-puk')}
                      >
                        Change unlock code…
                      </Button>
                    </>
                  }
                >
                  Use the unlock code (PUK) to set a new PIN, or set a new
                  unlock code.
                </InsetRow>
              </Inset>
            )}
          </div>
          <div
            ref={anchors.notifications}
            role="region"
            aria-labelledby="settings-notifications-label"
            tabIndex={-1}
          >
            <SectionLabel id="settings-notifications-label">
              Notifications
            </SectionLabel>
            <NotificationSettings />
          </div>
          <AboutSection
            snapshot={snapshot}
            bridge={bridge}
            appInfo={appInfo}
            anchor={anchors.about}
            onError={onError}
            onMessage={(text: string) => toasts.show(text)}
            onLock={onLock}
            agentLifecycle={agentLifecycle}
            onRetryAgent={onRetryAgent}
          />
          <SectionLabel className="danger-title">Danger zone</SectionLabel>
          <Inset className="settings-inset middle wide danger-box">
            <InsetRow
              className="dangerrow"
              label="Reset this Mac"
              action={
                <Button
                  size="sm"
                  variant="danger"
                  disabled={!snapshot.servers.length}
                  title={
                    snapshot.servers.length
                      ? undefined
                      : 'No server is configured on this Mac'
                  }
                  onClick={() => setSheet('reset-mac')}
                >
                  Reset this Mac…
                </Button>
              }
            >
              <small>
                Removes the local account keys, trust history, cache and
                unfinished operations this Mac holds for every server. Your
                accounts keep existing on their servers and other devices are
                untouched.
              </small>
            </InsetRow>
          </Inset>
        </div>
      </div>
      {sheet === 'passphrase' && passphrase ? (
        <PassphraseSheet
          bridge={bridge}
          store={passphrase.store}
          subtitle={accountSubtitle(snapshot, passphrase.store)}
          initialMode={passphrase.mode}
          onClose={() => {
            setSheet(null);
            setPassphrase(null);
          }}
          onDone={(message) => {
            setSheet(null);
            setPassphrase(null);
            void onRefresh(message).catch((error) => onMutationError(error));
          }}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
      {sheet === 'yubi' && addressed ? (
        <YubiActionSheet
          bridge={bridge}
          store={addressed}
          action={yubiAction}
          alias={enrolled?.alias ?? ''}
          onClose={() => setSheet(null)}
          onDone={async () => {
            setSheet(null);
            await onRefresh('Security key updated.');
          }}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
      {sheet === 'reset-mac' ? (
        <ResetMacSheet
          snapshot={snapshot}
          bridge={bridge}
          onClose={() => setSheet(null)}
          onDone={async (count) => {
            setSheet(null);
            await onRefreshSnapshot();
            await onRefresh(
              `Local state reset for ${plural(count, 'server')}. Verify each server before reconnecting.`,
            );
          }}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
    </>
  );
}

function AgentSection({
  snapshot,
  bridge,
  appInfo,
  onError,
  onMessage,
  agentLifecycle,
  onRetryAgent,
}: {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  appInfo: AppInfo | null;
  onError: (error: unknown) => void;
  onMessage: (message: string) => void;
  agentLifecycle: AgentLifecycle;
  onRetryAgent: () => Promise<void>;
}): ReactNode {
  const ready =
    snapshot.agent.state === 'ready' && agentLifecycle.state === 'ready';
  return (
    <>
      <InsetRow
        label="Agent"
        action={
          ready ? undefined : (
            <Button
              size="sm"
              variant="primary"
              onClick={() =>
                void onRetryAgent()
                  .then(() => onMessage('Connected to local agent.'))
                  .catch(onError)
              }
            >
              Retry connection
            </Button>
          )
        }
      >
        <span className={ready ? 'agent' : 'agent warn'}>
          <i />
          {snapshot.agent.state === 'bootstrap'
            ? 'Starting the FOKS agent'
            : agentLifecycleLabel(agentLifecycle)}
        </span>
        <small>The local background agent must be connected to use FOKS.</small>
      </InsetRow>
      <InsetRow
        label="Socket"
        valueClass="mono"
        action={
          appInfo ? (
            <Button
              size="sm"
              onClick={() =>
                void bridge
                  .copyText(appInfo.agentSocket)
                  .then(() => onMessage('Socket path copied'))
                  .catch(onError)
              }
            >
              Copy
            </Button>
          ) : undefined
        }
      >
        {appInfo?.agentSocket ?? 'Reading app info…'}
        <small>Local connection used by this desktop app.</small>
      </InsetRow>
    </>
  );
}

/**
 * About and This Mac: two sections, so two regions. What the application is
 * and what this Mac's copy of it can be moved to are different subjects, and
 * one region named About would name half of what it encloses.
 */
function AboutSection({
  snapshot,
  bridge,
  appInfo,
  anchor,
  onError,
  onMessage,
  onLock,
  agentLifecycle,
  onRetryAgent,
}: {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  appInfo: AppInfo | null;
  /** The `section=about` address lands on the first of the two. */
  anchor: RefObject<HTMLDivElement | null>;
  onError: (error: unknown) => void;
  onMessage: (message: string) => void;
  onLock: () => Promise<boolean>;
  agentLifecycle: AgentLifecycle;
  onRetryAgent: () => Promise<void>;
}): ReactNode {
  const maintenanceUnavailable = agentLifecycle.state !== 'ready';
  return (
    <>
      <div
        className="settings-section"
        ref={anchor}
        tabIndex={-1}
        role="region"
        aria-labelledby="settings-about-label"
      >
        <SectionLabel id="settings-about-label">About</SectionLabel>
        <Inset className="settings-inset middle wide">
          <InsetRow label="Version">
            FOKS Desktop {appInfo?.version ?? '…'}
            <small>Installed application version.</small>
          </InsetRow>
          <AgentSection
            snapshot={snapshot}
            bridge={bridge}
            appInfo={appInfo}
            onError={onError}
            onMessage={onMessage}
            agentLifecycle={agentLifecycle}
            onRetryAgent={onRetryAgent}
          />
          <InsetRow
            label="Lock application"
            action={
              <Button
                size="sm"
                icon="shield"
                onClick={() => {
                  void onLock().then(
                    (locked) => {
                      if (!locked)
                        onMessage(
                          'Application lock is not available on this system.',
                        );
                    },
                    (error) => onError(error),
                  );
                }}
              >
                Lock now
              </Button>
            }
          >
            Require your operating-system credentials before FOKS can read vault
            data again.
          </InsetRow>
        </Inset>
      </div>
      <div
        className="settings-section"
        role="region"
        aria-labelledby="settings-this-mac-label"
      >
        <SectionLabel id="settings-this-mac-label">This Mac</SectionLabel>
        <Inset className="settings-inset wide">
          <InsetRow
            label="Transfer FOKS state"
            action={
              <>
                <Button
                  size="sm"
                  disabled={maintenanceUnavailable}
                  onClick={() => {
                    void bridge.maintainClientState('export').catch(onError);
                  }}
                >
                  Export…
                </Button>
                <Button
                  size="sm"
                  disabled={maintenanceUnavailable}
                  onClick={() => {
                    void bridge.maintainClientState('import').catch(onError);
                  }}
                >
                  Import…
                </Button>
              </>
            }
          >
            <small>
              Encrypted backups copy device credentials. Use a new device for
              independent revocation.
            </small>
          </InsetRow>
          <InsetRow
            label="Verify imported accounts"
            action={
              <Button
                size="sm"
                disabled={maintenanceUnavailable}
                onClick={() => {
                  void bridge.maintainClientState('verify').catch(onError);
                }}
              >
                Verify online
              </Button>
            }
          >
            <small>
              Check current account and device authority before enabling
              imported state.
            </small>
          </InsetRow>
          <InsetRow
            label="Move FOKS data"
            action={
              <Button
                size="sm"
                disabled={maintenanceUnavailable}
                onClick={() => {
                  void bridge.relocateClientState().catch(onError);
                }}
              >
                Choose folder…
              </Button>
            }
          >
            <small>
              Move every profile and its credentials to another folder on this
              disk. FOKS verifies the move and restarts.
            </small>
          </InsetRow>
        </Inset>
      </div>
    </>
  );
}

/**
 * Reset this Mac: `describe_reset` and `reset_server` once per server.
 *
 * There is no command that spans profiles, so each server answers with its own
 * one-use token and each profile name is typed. A run that fails part way says
 * how far it got rather than pretending the rest happened.
 */
function ResetMacSheet({
  snapshot,
  bridge,
  onClose,
  onDone,
  onError,
}: {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  onClose: () => void;
  onDone: (count: number) => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const servers = snapshot.servers;
  const [previews, setPreviews] = useState<Map<string, ResetPreview>>(
    new Map(),
  );
  const [failures, setFailures] = useState<Map<string, string>>(new Map());
  const [typed, setTyped] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(true);

  const load = useCallback((): void => {
    setLoading(true);
    setPreviews(new Map());
    setFailures(new Map());
    void (async () => {
      const found = new Map<string, ResetPreview>();
      const failed = new Map<string, string>();
      for (const server of servers) {
        try {
          const preview = await enqueueProfileWork(bridge, server.id, () =>
            bridge.describeReset(server.id),
          );
          if (preview.profile !== server.id)
            throw new Error('describe_reset returned a different profile.');
          found.set(server.id, preview);
        } catch (error) {
          failed.set(server.id, normalizeCommandError(error).message);
        }
      }
      setPreviews(found);
      setFailures(failed);
      setLoading(false);
    })();
  }, [bridge, servers]);
  useEffect(load, [load]);

  const ready =
    !loading &&
    servers.length > 0 &&
    servers.every(
      (server) => previews.has(server.id) && typed[server.id] === server.id,
    );
  const stores = accountStores(snapshot);

  // Each server may configure a different reset preview token TTL. Display the
  // expiration notice only after all server previews have responded, without
  // assuming a fallback duration.
  const lifetimes = servers.map(
    (server) => previews.get(server.id)?.expiresInSeconds,
  );
  const answered =
    servers.length > 0 && lifetimes.every((seconds) => seconds !== undefined);
  const agreed =
    answered && new Set(lifetimes).size === 1 ? lifetimes[0] : undefined;
  const expiry = !answered
    ? null
    : agreed !== undefined
      ? `Each reset confirmation expires in ${agreed} seconds and can be used once. Reopen this dialog to generate new ones.`
      : `Each reset confirmation can be used once and has a server-specific expiration duration: ${servers
          .map(
            (server) =>
              `${serverDisplayName(server)} (${previews.get(server.id)?.expiresInSeconds} seconds)`,
          )
          .join(', ')}. Reopen this dialog to generate new confirmations.`;

  return (
    <SheetDialog
      width="wide"
      danger
      dismissible={!busy}
      onClose={() => {
        if (busy) return;
        onClose();
      }}
      title="Reset this Mac?"
      subtitle="Delete local account keys and reset server trust for every server on this Mac"
      glyph={
        <span className="server-mark danger">
          <Icon name="trash" />
        </span>
      }
      footer={
        <>
          <Button disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="danger"
            disabled={!ready || busy}
            onClick={() => {
              setBusy(true);
              void (async () => {
                let done = 0;
                try {
                  for (const server of servers) {
                    const preview = previews.get(server.id);
                    if (!preview) break;
                    await bridge.resetServer(
                      server.id,
                      server.id,
                      preview.token,
                    );
                    done += 1;
                  }
                  await onDone(done);
                } catch (error) {
                  onError(error);
                  // Every token is spent or stale once a run has started.
                  load();
                } finally {
                  setBusy(false);
                }
              })();
            }}
          >
            Reset this Mac
          </Button>
        </>
      }
    >
      <p>
        This permanently deletes local account keys. Server data is not deleted,
        but you can permanently lose access to it without another enrolled
        device, a paper key you wrote down, or a usable external backup of your
        local state. Your account passphrase alone cannot restore the deleted
        keys.
      </p>
      <Inset>
        <InsetRow label="Unaffected">
          Your accounts on their servers, and every other device. Only what this
          Mac holds is erased.
        </InsetRow>
        <InsetRow label="Accounts on this Mac">
          {stores.length
            ? stores
                .map(
                  (store) =>
                    `${usernameOf(snapshot, store) ?? store.account} (${store.account})`,
                )
                .join(', ')
            : 'None'}
        </InsetRow>
      </Inset>
      {servers.map((server) => {
        const preview = previews.get(server.id);
        const failure = failures.get(server.id);
        return (
          <div key={server.id}>
            <SectionLabel>
              {serverDisplayName(server)} · {server.id}
            </SectionLabel>
            <Inset>
              {failure ? (
                <InsetRow label="Preview">
                  <span className="danger-title">{failure}</span>
                </InsetRow>
              ) : preview ? (
                <>
                  <InsetRow label="Discarded operations">
                    {preview.resumables.length
                      ? preview.resumables
                          .map(
                            (row) =>
                              `${row.kind} · ${row.alias}${row.target ? ` · ${row.target}` : ''}`,
                          )
                          .join(', ')
                      : 'No resumable operations found.'}
                  </InsetRow>
                  <InsetRow label="Data to erase">
                    {preview.artifacts.length
                      ? preview.artifacts
                          .map(
                            (row) =>
                              `${row.kind} · ${row.entries} entries · ${row.bytes.toLocaleString()} bytes`,
                          )
                          .join(', ')
                      : 'No local data to erase.'}
                  </InsetRow>
                </>
              ) : (
                <InsetRow label="Preview">Loading…</InsetRow>
              )}
              <InsetRow label="Confirm">
                <input
                  value={typed[server.id] ?? ''}
                  disabled={!preview}
                  onChange={(event) =>
                    setTyped((current) => ({
                      ...current,
                      [server.id]: event.target.value,
                    }))
                  }
                  placeholder={`Type "${server.id}" to confirm`}
                />
              </InsetRow>
            </Inset>
          </div>
        );
      })}
      {servers.length ? null : (
        <Band label="No servers on this Mac">
          There is nothing for this reset to erase.
        </Band>
      )}
      {expiry ? <p className="hint">{expiry}</p> : null}
    </SheetDialog>
  );
}
