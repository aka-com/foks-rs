import { localAliasOf } from '../model';
/**
 * The Settings tab: a sub-navigation of three pages, held on screen beside
 * whichever one is open.
 *
 * Account, the first page and the one the tab opens on, is one account's
 * profile: the account the address's `store` names. At its bottom are the
 * servers this Mac talks to. Each server opens its own page, where its security
 * keys are managed. Preferences contains local desktop alert and appearance settings.
 * Device contains the application version and lock, the agent and its socket,
 * local FOKS data operations, and the device-wide reset. A `section=` address
 * opens its page; `profile=` opens a server page under Account.
 *
 * Account draws its own header — the account's mark, username and server —
 * and its own tab panel, and owns its sheets. The other two share the
 * header and panel drawn here.
 */

import { useCallback, useEffect, useState } from 'react';
import type { ReactNode } from 'react';
import { useToast } from '/kit/toasts';
import { enqueueProfileWork, normalizeCommandError } from '../bridge';
import type {
  AgentProcessInfo,
  AppInfo,
  Bridge,
  ResetPreview,
} from '../bridge';
import {
  Band,
  Button,
  Icon,
  Inset,
  InsetRow,
  SectionLabel,
  SheetDialog,
  Tabs,
  tabId,
  tabPanelId,
} from '../components';
import {
  SETTINGS_SECTIONS,
  SETTINGS_SECTION_LABEL,
  settingsSectionOf,
} from '../location';
import type { Location, NavigateOptions, SettingsSection } from '../location';
import { useSheetGuard } from '../navigation-guard';
import { useMetadataQuery, useMetadataRepository } from '../query-hooks';
import { appInfoQuery } from '../resources/application';
import { accountStores, plural, serverDisplayName, usernameOf } from '../model';
import type { AgentSnapshot, Server } from '../model';
import { PageHeader } from '../shell/page-header';
import {
  RAIL_COLORS,
  applyRailColor,
  rememberRailColor,
  storedRailColor,
} from '../rail-theme';
import type { MutationFailureHandler } from '../mutation-recovery';
import { agentLifecycleLabel, type AgentLifecycle } from '../agent-lifecycle';
import { NotificationSettings } from '../chat/notification-provider';
import { ServersSection } from './servers-screen';
import { AccountSection } from './account-section';

export interface SettingsScreenProps {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  location: Extract<Location, { kind: 'settings' }>;
  scene: string;
  onNavigate: (location: Location, options?: NavigateOptions) => void;
  onRefresh: (message: string) => Promise<void>;
  onRefreshSnapshot: () => Promise<AgentSnapshot>;
  onError: (error: unknown) => void;
  onMutationError: MutationFailureHandler;
  onLock: () => Promise<boolean>;
  agentLifecycle: AgentLifecycle;
  onRetryAgent: () => Promise<void>;
}

/** A sheet this page owns. Add a server and server removal are the
 *  embedded server section's own. */
type Sheet = 'reset-mac' | null;

/** The base the sub-navigation's tab and panel ids are derived from. */
const SETTINGS_TABS = 'settings-sections';

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
  const [sheet, setSheet] = useState<Sheet>(null);
  const repository = useMetadataRepository(bridge);
  const { data: appInfo } = useMetadataQuery(appInfoQuery(repository, bridge), {
    onError,
  });

  const section: SettingsSection = settingsSectionOf(location);

  const serverPanel = {
    id: tabPanelId(SETTINGS_TABS, 'account'),
    labelledBy: tabId(SETTINGS_TABS, 'account'),
  };
  const selectedServer = location.profile
    ? snapshot.servers.find((server) => server.id === location.profile)
    : undefined;
  const serversSection = (
    <ServersSection
      snapshot={snapshot}
      bridge={bridge}
      store={location.store}
      scene={enteredScene}
      onNavigate={onNavigate}
      onRefresh={onRefresh}
      onError={onError}
      onMutationError={onMutationError}
    />
  );

  const page: ReactNode =
    section === 'account' ? null : section === 'preferences' ? (
      <PreferencesSection />
    ) : (
      <DeviceSection
        snapshot={snapshot}
        bridge={bridge}
        appInfo={appInfo ?? null}
        onError={onError}
        onMessage={(text: string) => toasts.show(text)}
        onLock={onLock}
        agentLifecycle={agentLifecycle}
        onRetryAgent={onRetryAgent}
        serversCount={snapshot.servers.length}
        onReset={() => setSheet('reset-mac')}
      />
    );

  return (
    <div className="settingslayout">
      <nav className="side subnav-side" aria-label="Settings sections">
        <Tabs
          orientation="vertical"
          label="Settings sections"
          idBase={SETTINGS_TABS}
          value={section}
          onChange={(next) => {
            // Settings sections are sibling views within this tab and replace
            // current content without adding history entries. Selecting a
            // section opens its root view without a server profile.
            onNavigate(
              {
                kind: 'settings',
                ...(location.store ? { store: location.store } : {}),
                section: next,
              },
              { replace: true },
            );
          }}
          items={SETTINGS_SECTIONS.map((id) => ({
            id,
            label: SETTINGS_SECTION_LABEL[id],
          }))}
        />
      </nav>
      <div className="subnav-page">
        {section === 'account' && selectedServer ? (
          <ServersSection
            snapshot={snapshot}
            bridge={bridge}
            profile={selectedServer.id}
            store={location.store}
            scene={enteredScene}
            panel={serverPanel}
            onNavigate={onNavigate}
            onRefresh={onRefresh}
            onError={onError}
            onMutationError={onMutationError}
          />
        ) : section === 'account' ? (
          <AccountSection
            snapshot={snapshot}
            bridge={bridge}
            location={location}
            panel={serverPanel}
            onNavigate={onNavigate}
            onRefresh={onRefresh}
            onRefreshSnapshot={onRefreshSnapshot}
            onError={onError}
            onMutationError={onMutationError}
            serverSection={serversSection}
          />
        ) : (
          <>
            <PageHeader ruled title={SETTINGS_SECTION_LABEL[section]} />
            {/* Fixed IDs associate each tab button with its tabpanel. */}
            <div
              className="body"
              role="tabpanel"
              id={tabPanelId(SETTINGS_TABS, section)}
              aria-labelledby={tabId(SETTINGS_TABS, section)}
            >
              <div className="settings-main">{page}</div>
            </div>
          </>
        )}
      </div>
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
    </div>
  );
}

/** Locally stored desktop alert and appearance preferences. */
function PreferencesSection(): ReactNode {
  return (
    <>
      <SectionLabel>Desktop alerts</SectionLabel>
      <NotificationSettings />
      <SectionLabel>Appearance</SectionLabel>
      <RailColorPicker />
    </>
  );
}

/**
 * The rail color picker. Applies each choice at once, so the rail beside
 * this page shows it.
 */
function RailColorPicker(): ReactNode {
  const [color, setColor] = useState(storedRailColor);
  const choose = (id: string): void => {
    setColor(id);
    applyRailColor(id);
    rememberRailColor(id);
  };
  return (
    <Inset className="settings-inset middle wide">
      <InsetRow label="Sidebar color">
        <div className="swatches" role="radiogroup" aria-label="Sidebar color">
          {RAIL_COLORS.map((entry) => (
            <button
              key={entry.id}
              type="button"
              role="radio"
              className={entry.id === color ? 'swatch on' : 'swatch'}
              aria-checked={entry.id === color}
              aria-label={entry.label}
              title={entry.label}
              onClick={() => choose(entry.id)}
            >
              <i style={{ background: entry.hex }} aria-hidden="true" />
              <span>{entry.label}</span>
            </button>
          ))}
        </div>
      </InsetRow>
    </Inset>
  );
}

/**
 * Device: what the application and its agent are, the local FOKS data
 * operations, and the one reset that acts on this Mac rather than on an
 * account. The reset stays visually marked as destructive, in its own danger
 * zone at the foot of this page rather than of everything Settings holds.
 */
function DeviceSection({
  snapshot,
  bridge,
  appInfo,
  onError,
  onMessage,
  onLock,
  agentLifecycle,
  onRetryAgent,
  serversCount,
  onReset,
}: {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  appInfo: AppInfo | null;
  onError: (error: unknown) => void;
  onMessage: (message: string) => void;
  onLock: () => Promise<boolean>;
  agentLifecycle: AgentLifecycle;
  onRetryAgent: () => Promise<void>;
  serversCount: number;
  onReset: () => void;
}): ReactNode {
  const ready =
    snapshot.agent.state === 'ready' && agentLifecycle.state === 'ready';
  const maintenanceUnavailable = agentLifecycle.state !== 'ready';
  // The process on the socket: who started it decides whether maintenance can
  // stop it, and Status says so. Re-read whenever the agent's state settles.
  const [process, setProcess] = useState<AgentProcessInfo | null>(null);
  useEffect(() => {
    let alive = true;
    setProcess(null);
    if (!ready) return;
    void bridge
      .agentProcessInfo()
      .then((info) => {
        if (alive) setProcess(info);
      })
      .catch(() => {
        // Descriptive only: a failed read leaves the row without its detail.
      });
    return () => {
      alive = false;
    };
  }, [bridge, ready, agentLifecycle.state]);
  const foreign = process !== null && process.pid !== null && !process.owned;
  // The restart sheet: opened from the Status row, or by a maintenance action
  // that the agent's ownership refused, in which case it carries the action
  // to run once the agent is ours.
  const [restart, setRestart] = useState<RestartRequest | null>(null);
  const maintain = (
    purpose: RestartPurpose,
    run: () => Promise<unknown>,
  ): void => {
    void run().catch((error: unknown) => {
      if (isExternalAgent(error)) setRestart({ purpose, then: run });
      else onError(error);
    });
  };
  return (
    <>
      {restart ? (
        <RestartAgentSheet
          bridge={bridge}
          request={restart}
          process={process}
          onClose={() => setRestart(null)}
          onError={onError}
        />
      ) : null}
      <SectionLabel>Application</SectionLabel>
      <Inset className="settings-inset middle wide">
        <InsetRow label="Version">
          FOKS Desktop {appInfo?.version ?? '…'}
        </InsetRow>
        <InsetRow
          label="Lock"
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
      <SectionLabel>Agent</SectionLabel>
      {/* Top-align the multi-line Status row with its primary status line.
          Single-line Socket rows use vertical centering. */}
      <Inset className="settings-inset wide">
        <InsetRow
          label="Status"
          action={
            ready ? (
              <Button
                size="sm"
                disabled={maintenanceUnavailable}
                onClick={() => setRestart({ purpose: 'restart' })}
              >
                Restart…
              </Button>
            ) : (
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
          {ready ? null : (
            <small>
              The local background agent must be connected to use FOKS.
            </small>
          )}
          {foreign ? (
            <small>
              Not started by this app. Transfer and move restart it first.
            </small>
          ) : null}
        </InsetRow>
        <InsetRow
          className="line"
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
        </InsetRow>
      </Inset>
      <SectionLabel>FOKS data</SectionLabel>
      <Inset className="settings-inset wide">
        <InsetRow
          label="Transfer"
          action={
            <>
              <Button
                size="sm"
                disabled={maintenanceUnavailable}
                onClick={() =>
                  maintain('export', () => bridge.maintainClientState('export'))
                }
              >
                Export…
              </Button>
              <Button
                size="sm"
                disabled={maintenanceUnavailable}
                onClick={() =>
                  maintain('import', () => bridge.maintainClientState('import'))
                }
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
              onClick={() =>
                maintain('verify', () => bridge.maintainClientState('verify'))
              }
            >
              Verify online
            </Button>
          }
        >
          <small>
            Check current account and device authority before enabling imported
            state.
          </small>
        </InsetRow>
        <InsetRow
          label="Move FOKS data"
          action={
            <Button
              size="sm"
              disabled={maintenanceUnavailable}
              onClick={() =>
                maintain('relocate', () => bridge.relocateClientState())
              }
            >
              Choose folder…
            </Button>
          }
        >
          <small>
            Move every profile and its credentials to another folder on this
            device. Requires restarting the application.
          </small>
        </InsetRow>
      </Inset>
      <SectionLabel className="danger-title">Danger zone</SectionLabel>
      <Inset className="settings-inset middle wide danger-box">
        <InsetRow
          className="dangerrow"
          label="Reset this Mac"
          action={
            <Button
              size="sm"
              variant="danger"
              disabled={!serversCount}
              title={
                serversCount
                  ? undefined
                  : 'No server is configured on this device'
              }
              onClick={onReset}
            >
              Reset this Mac…
            </Button>
          }
        >
          <small>
            Deletes local account keys, trust history, cached state, and pending
            operations. Remote accounts and other enrolled devices are not
            affected.
          </small>
        </InsetRow>
      </Inset>
    </>
  );
}

/** Why the agent is being restarted: for its own sake, or to enable a
 *  maintenance action the agent's ownership refused. */
type RestartPurpose = 'restart' | 'export' | 'import' | 'verify' | 'relocate';

interface RestartRequest {
  purpose: RestartPurpose;
  /** The refused action, run again once the agent is this app's. */
  then?: () => Promise<unknown>;
}

const RESTART_COPY: Readonly<
  Record<Exclude<RestartPurpose, 'restart'>, { title: string; enable: string }>
> = {
  export: { title: 'Restart the agent to export?', enable: 'export' },
  import: { title: 'Restart the agent to import?', enable: 'import' },
  verify: {
    title: 'Restart the agent to verify?',
    enable: 'verification',
  },
  relocate: {
    title: 'Restart the agent to move FOKS data?',
    enable: 'the move',
  },
};

function isExternalAgent(error: unknown): boolean {
  return normalizeCommandError(error).code === 'external-agent';
}

/** "Started today, 16:26" / "Started yesterday, 16:26" / "Started Sep 16, 16:26". */
function startedLabel(startedAt: number, now = Date.now()): string {
  const started = new Date(startedAt * 1000);
  const time = started.toLocaleTimeString(undefined, {
    hour: '2-digit',
    minute: '2-digit',
    hourCycle: 'h23',
  });
  const day = (date: Date): number =>
    new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
  const days = Math.round((day(new Date(now)) - day(started)) / 86_400_000);
  const when =
    days === 0
      ? 'today'
      : days === 1
        ? 'yesterday'
        : started.toLocaleDateString(undefined, {
            month: 'short',
            day: 'numeric',
          });
  return `Started ${when}, ${time}`;
}

/**
 * Restart the agent: stop the process on the socket and start this app's
 * own. A plain restart asks once; a restart in service of a maintenance
 * action says which, and runs that action once the new agent is up.
 */
function RestartAgentSheet({
  bridge,
  request,
  process,
  onClose,
  onError,
}: {
  bridge: Bridge;
  request: RestartRequest;
  process: AgentProcessInfo | null;
  onClose: () => void;
  onError: (error: unknown) => void;
}): ReactNode {
  const [busy, setBusy] = useState(false);
  const copy =
    request.purpose === 'restart' ? null : RESTART_COPY[request.purpose];
  const plain = copy === null;
  const running =
    process?.pid !== null && process?.pid !== undefined
      ? `foks-agent (PID ${process.pid})` +
        (process.startedAt !== null
          ? ` · ${startedLabel(process.startedAt)}`
          : '')
      : 'No agent is answering on the socket.';
  return (
    <SheetDialog
      width="wide"
      dismissible={!busy}
      onClose={() => {
        if (!busy) onClose();
      }}
      title={copy ? copy.title : 'Restart the agent?'}
      glyph={
        <span className="server-mark">
          <Icon name="again" />
        </span>
      }
      footer={
        <>
          <Button disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            busy={busy}
            disabled={busy}
            onClick={() => {
              setBusy(true);
              void (async () => {
                try {
                  // A plain restart of an agent we own needs no takeover; a
                  // foreign one does, and so does any restart made to enable
                  // an action the ownership check refused.
                  await bridge.restartAgent(
                    !plain || Boolean(process && !process.owned),
                  );
                  // The shell toasts the restart's outcome from the
                  // maintenance event; only the follow-on action is this
                  // sheet's to run.
                  onClose();
                  if (request.then) await request.then();
                } catch (error) {
                  onError(error);
                  onClose();
                }
              })();
            }}
          >
            {busy ? 'Restarting…' : 'Restart agent'}
          </Button>
        </>
      }
    >
      <p>
        {copy
          ? `The running foks-agent was not started by this app. Stop it, and start a new agent, to enable ${copy.enable}?`
          : 'FOKS will stop the local agent and start it again. This will take a few seconds.'}
      </p>
      <Inset>
        <InsetRow label="Running agent">{running}</InsetRow>
        {process?.executable ? (
          <InsetRow label="Launched from" valueClass="mono">
            {process.executable}
          </InsetRow>
        ) : null}
        {plain ? (
          <InsetRow label="In progress">Nothing in progress.</InsetRow>
        ) : null}
      </Inset>
    </SheetDialog>
  );
}

/**
 * Reset this Mac: `describe_reset` and `reset_server` once per server.
 *
 * There is no command that spans profiles, so each server answers with its own
 * one-use token and each profile name is typed. A run that fails part way says
 * how far it got rather than pretending the rest happened.
 *
 * A reset is the way out of local state that no longer works, so one server's
 * preview failing does not hold the others: the run covers every server whose
 * preview answered, says which it left out, and offers each of those its own
 * retry. A preview that could not read this Mac's credentials still answers,
 * saying so, and the reset it authorizes erases what is on disk.
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

  const [retrying, setRetrying] = useState<Set<string>>(new Set());

  const preview = useCallback(
    async (server: Server): Promise<ResetPreview> => {
      const preview = await enqueueProfileWork(bridge, server.id, () =>
        bridge.describeReset(server.id),
      );
      if (preview.profile !== server.id)
        throw new Error('describe_reset returned a different profile.');
      return preview;
    },
    [bridge],
  );
  const load = useCallback((): void => {
    setLoading(true);
    setPreviews(new Map());
    setFailures(new Map());
    void (async () => {
      const found = new Map<string, ResetPreview>();
      const failed = new Map<string, string>();
      for (const server of servers) {
        try {
          found.set(server.id, await preview(server));
        } catch (error) {
          failed.set(server.id, normalizeCommandError(error).message);
        }
      }
      setPreviews(found);
      setFailures(failed);
      setLoading(false);
    })();
  }, [preview, servers]);
  useEffect(load, [load]);
  // One server's preview again, leaving the others' answers and typed names
  // where they are.
  const retry = (server: Server): void => {
    setRetrying((current) => new Set(current).add(server.id));
    void (async () => {
      try {
        const answer = await preview(server);
        setPreviews((current) => new Map(current).set(server.id, answer));
        setFailures((current) => {
          const next = new Map(current);
          next.delete(server.id);
          return next;
        });
      } catch (error) {
        setFailures((current) =>
          new Map(current).set(server.id, normalizeCommandError(error).message),
        );
      } finally {
        setRetrying((current) => {
          const next = new Set(current);
          next.delete(server.id);
          return next;
        });
      }
    })();
  };

  // Reset executes sequentially per server using single-use confirmation
  // tokens. Navigating away during execution would prevent status updates.
  // Typed server names confirm execution and are not persisted.
  useSheetGuard(
    busy
      ? { verdict: 'refuse', reason: 'Wait for the reset to finish.' }
      : null,
  );

  // The servers whose preview answered are the run; a server whose preview
  // did not is left out and said so, not a reason to hold the rest.
  const included = servers.filter((server) => previews.has(server.id));
  const excluded = servers.filter((server) => !previews.has(server.id));
  const ready =
    !loading &&
    retrying.size === 0 &&
    included.length > 0 &&
    included.every((server) => typed[server.id] === server.id);
  const stores = accountStores(snapshot);

  // Each server may configure a different reset preview token TTL. Display the
  // expiration notice only after all server previews have responded, without
  // assuming a fallback duration.
  const lifetimes = included.map(
    (server) => previews.get(server.id)?.expiresInSeconds,
  );
  const answered =
    !loading &&
    included.length > 0 &&
    lifetimes.every((seconds) => seconds !== undefined);
  const agreed =
    answered && new Set(lifetimes).size === 1 ? lifetimes[0] : undefined;
  const expiry = !answered
    ? null
    : agreed !== undefined
      ? `Confirmations expire in ${agreed} seconds.`
      : `Confirmations expire per server: ${included
          .map(
            (server) =>
              `${serverDisplayName(server)} (${previews.get(server.id)?.expiresInSeconds} seconds)`,
          )
          .join(', ')}.`;

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
                  for (const server of included) {
                    const preview = previews.get(server.id);
                    if (!preview) continue;
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
            {excluded.length && included.length
              ? `Reset ${included.length} of ${servers.length} servers`
              : 'Reset this Mac'}
          </Button>
        </>
      }
    >
      <p>
        Deletes this Mac's account keys. Your data stays on the server, but
        without another device or a paper key you cannot get back into the
        account. The passphrase alone is not enough.
      </p>
      <Inset>
        <InsetRow label="Unaffected">
          Remote accounts and all other enrolled devices remain active. Only
          data stored locally on this Mac is erased.
        </InsetRow>
        <InsetRow label="Accounts on this Mac">
          {stores.length
            ? stores
                .map(
                  (store) =>
                    `${usernameOf(snapshot, store) ?? store.account} (${localAliasOf(snapshot, store)})`,
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
                <InsetRow
                  label="Preview"
                  action={
                    <Button
                      size="sm"
                      disabled={busy || retrying.has(server.id)}
                      busy={retrying.has(server.id)}
                      onClick={() => retry(server)}
                    >
                      Retry preview
                    </Button>
                  }
                >
                  <span className="danger-title">{failure}</span>{' '}
                  {loading ? null : 'Not included in this reset.'}
                </InsetRow>
              ) : preview ? (
                <>
                  {preview.credentialsUnavailable ? (
                    <InsetRow label="Credentials">
                      <span className="danger-title">
                        {preview.credentialsUnavailable}
                      </span>{' '}
                      The reset erases this server’s local data and leaves the
                      credential records it cannot read.
                    </InsetRow>
                  ) : null}
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
      {!loading && excluded.length ? (
        <Band label="Not every server answered">
          {included.length
            ? `${plural(excluded.length, 'server')} could not be previewed and ${excluded.length === 1 ? 'is' : 'are'} not included. Retry ${excluded.length === 1 ? 'its' : 'their'} preview, or reset the rest now and ${excluded.length === 1 ? 'that server' : 'those servers'} later.`
            : 'No server could be previewed. Retry each preview to reset it.'}
        </Band>
      ) : null}
      {expiry ? <p className="hint">{expiry}</p> : null}
    </SheetDialog>
  );
}
