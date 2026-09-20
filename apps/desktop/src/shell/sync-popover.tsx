/**
 * Refresh status, behind the top bar's refresh button.
 *
 * The scheduler observes several jobs per server (the catalog read, the
 * connectivity probe, team discovery) and two for this Mac (the registry and
 * account metadata). One root cause — a missing keystore record, an
 * unreachable host — fails all of them with the same message, so the
 * observations are grouped by server and duplicate errors are collapsed. Each
 * server lists its jobs with their most recent success, how long that run
 * took and their next-run times.
 * Failures add a summary and recovery actions. A single server is always
 * expanded; when several servers are present, failed or busy servers start expanded
 * and the others start collapsed. Copy diagnostics retains the ungrouped
 * observations.
 */

import {
  useEffect,
  useId,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react';
import type { ReactNode, RefObject } from 'react';
import { Popover } from '/kit/overlay-primitives';
import type { AgentSnapshot, CatalogFreshnessEntry, Server } from '../model';
import { chatAvailable, serverDisplayName } from '../model';
import { useSidebarInbox } from '../chat/inbox-provider';
import {
  useShellDeviceMetadata,
  emptyDeviceMetadata,
  noDeviceSubscription,
  type DeviceMetadataStatus,
} from '../device-metadata';
import { formatMilliseconds, formatTimings } from '../diagnostics/format';
import { diagnosticLog, type TimingEvent } from '../diagnostics/log';
import type { DesktopReconciliation } from '../desktop-reconciliation';
import type {
  ReconciliationKind,
  ReconciliationSnapshot,
} from '../scheduling/reconciliation';
import {
  profileRefreshKey,
  profileConnectivityKey,
} from '../desktop-reconciliation';
import { Button, Icon } from '../components';
import { connectionSecurityFailure } from '../profile-connectivity';
import { normalizeCommandError } from '../bridge';

export type SyncState = 'ok' | 'refreshing' | 'failed' | 'unknown';

/** One of the scheduler's jobs on a server or on this Mac, as a line. */
export interface SyncJobSummary {
  kind: ReconciliationKind | 'chat' | 'devices';
  label: string;
  state: SyncState;
  /** Full status sentence exposed as the row's tooltip. */
  detail: string;
  /** Scheduler timestamps, in milliseconds since the Unix epoch. */
  lastSuccessAt?: number;
  lastAttemptAt?: number;
  nextAttemptAt?: number;
  /** Whether automatic retries are paused pending a manual refresh. */
  paused: boolean;
  /** How long the job's last completed run took, in milliseconds. */
  lastMilliseconds?: number;
}

export interface SyncServerSummary {
  /** The server id, or `null` for the observations that belong to this Mac. */
  id: string | null;
  name: string;
  state: SyncState;
  /** One sentence: what failed, what is shown instead, what happens next. */
  message: string;
  lastSuccessAt?: number;
  /** A Reconnect action is offered for this server. */
  canReconnect: boolean;
  reconnectDisabled: boolean;
  reconnecting: boolean;
  /** The jobs behind the row, in a fixed order, one line each. */
  jobs: SyncJobSummary[];
}

export interface SyncSummary {
  refreshing: boolean;
  /** App-wide work not represented by a server's three scheduler rows. */
  activities: readonly { id: string; label: string; startedAt?: number }[];
  failed: boolean;
  servers: SyncServerSummary[];
  /** The ungrouped observations, one line each, for Copy diagnostics. */
  diagnostics: string[];
}

function timeOf(seconds: number | undefined): string {
  return seconds === undefined
    ? ''
    : new Date(seconds * 1_000).toLocaleTimeString([], {
        hour: 'numeric',
        minute: '2-digit',
      });
}

function observationLabel(kind: string): string {
  return kind === 'discovery'
    ? 'Teams'
    : kind === 'metadata'
      ? 'Account metadata'
      : kind === 'connectivity'
        ? 'Connectivity'
        : kind === 'registry'
          ? 'Profile inventory'
          : 'Catalog';
}

/** Display order for jobs within each server. */
const JOB_ORDER: readonly ReconciliationKind[] = [
  'catalog',
  'connectivity',
  'discovery',
  'registry',
  'metadata',
];

/** A scheduler clock time, in milliseconds, as a clock time. */
function clockTime(milliseconds: number | undefined): string {
  return milliseconds === undefined ? '' : timeOf(milliseconds / 1_000);
}

/**
 * How long the job's last run took, as the row and its sentence state it.
 * Taken from the scheduler's own snapshot, so a cleared timing log does not
 * take the duration off the row with it.
 */
function runDuration(milliseconds: number | undefined): string {
  return milliseconds === undefined ? '' : formatMilliseconds(milliseconds);
}

/** The same duration as its own sentence, where no success time carries it. */
function lastRun(duration: string): string {
  return duration ? `Last run ${duration}.` : '';
}

function jobSummary(
  kind: ReconciliationKind,
  snapshot: Readonly<ReconciliationSnapshot>,
): SyncJobSummary {
  const label = observationLabel(kind);
  const next =
    snapshot.nextAttemptAt !== undefined
      ? `Next at ${clockTime(snapshot.nextAttemptAt)}.`
      : snapshot.paused
        ? 'Paused until Refresh.'
        : '';
  const succeeded =
    snapshot.lastSuccessAt !== undefined
      ? `Last succeeded ${clockTime(snapshot.lastSuccessAt)}.`
      : '';
  const line = (parts: readonly string[]) => parts.filter(Boolean).join(' ');
  const ran = runDuration(snapshot.lastMilliseconds);
  const times = {
    lastSuccessAt: snapshot.lastSuccessAt,
    lastAttemptAt: snapshot.lastAttemptAt,
    nextAttemptAt: snapshot.nextAttemptAt,
    paused: Boolean(snapshot.paused),
    lastMilliseconds: snapshot.lastMilliseconds,
  };
  if (snapshot.error) {
    const cause = normalizeCommandError(snapshot.error).message.replace(
      /\.+$/,
      '',
    );
    return {
      kind,
      label,
      state: snapshot.refreshing ? 'refreshing' : 'failed',
      detail: line([
        snapshot.refreshing ? 'Retrying now.' : '',
        `${cause}.`,
        succeeded,
        lastRun(ran),
        next,
      ]),
      ...times,
    };
  }
  if (snapshot.refreshing)
    return {
      kind,
      label,
      state: 'refreshing',
      detail: line(['Running now.', succeeded]),
      ...times,
    };
  if (snapshot.lastSuccessAt !== undefined)
    return {
      kind,
      label,
      state: 'ok',
      detail: line([
        `Succeeded ${clockTime(snapshot.lastSuccessAt)}${ran ? ` in ${ran}` : ''}.`,
        next,
      ]),
      ...times,
    };
  return {
    kind,
    label,
    state: 'unknown',
    detail: line([
      snapshot.lastAttemptAt !== undefined
        ? `Attempted ${clockTime(snapshot.lastAttemptAt)}, not yet successful.`
        : 'Not yet run.',
      lastRun(ran),
      next,
    ]),
    ...times,
  };
}

function jobSummaries(
  observations: readonly {
    kind: ReconciliationKind;
    snapshot: Readonly<ReconciliationSnapshot>;
  }[],
): SyncJobSummary[] {
  return [...observations]
    .sort((a, b) => JOB_ORDER.indexOf(a.kind) - JOB_ORDER.indexOf(b.kind))
    .map((observation) => jobSummary(observation.kind, observation.snapshot));
}

function failureSentence(
  messages: readonly string[],
  lastSuccessAt: number | undefined,
  paused: boolean,
): string {
  const cause = messages.join('; ').replace(/\.+$/, '');
  const shown =
    lastSuccessAt === undefined
      ? 'No successful refresh yet.'
      : `Showing data from ${timeOf(lastSuccessAt)}.`;
  const next = paused
    ? 'Automatic refresh is paused; use Refresh to retry.'
    : 'Retrying automatically.';
  return `${cause}. ${shown} ${next}`;
}

/** Format one catalog observation for copied diagnostics. */
function freshnessLine(entry: CatalogFreshnessEntry | undefined): string {
  if (!entry) return 'No refresh observation yet';
  const successful =
    entry.lastSuccessAt === undefined
      ? 'No successful refresh'
      : `Last successful refresh: ${new Date(entry.lastSuccessAt * 1_000).toLocaleTimeString()}`;
  if (entry.error)
    return `${successful}. Refresh failed: ${entry.error.message}`;
  // Freshness flags describe a published partial, not whether its owner is
  // still alive. Live work is reported separately below.
  return successful;
}

function reconnectDisabledFor(server: Server): boolean {
  return (
    server.trust.status === 'blocked' ||
    server.restrictions.some(
      (entry) =>
        entry.kind === 'schema-incompatible' ||
        entry.kind === 'import-verification-required',
    ) ||
    connectionSecurityFailure(server.connectivity)?.code ===
      'saved-trust-missing'
  );
}

export function summarizeSync(
  snapshot: AgentSnapshot,
  service: DesktopReconciliation,
  inbox?: ReturnType<typeof useSidebarInbox>,
  devices?: readonly DeviceMetadataStatus[],
): SyncSummary {
  const observations = service.scheduler.observations();
  const diagnostics: string[] = [];
  const catalogAttempt = snapshot.catalogFreshness?.attempt;
  if (catalogAttempt)
    diagnostics.push(`Catalog refresh: ${freshnessLine(catalogAttempt)}`);
  const servers: SyncServerSummary[] = snapshot.servers.map((server) => {
    const name = serverDisplayName(server);
    const entry = snapshot.catalogFreshness?.profiles[server.id];
    const attempt = service.scheduler.snapshot(
      profileRefreshKey(snapshot, server.id),
    );
    const connectivity = service.scheduler.snapshot(
      profileConnectivityKey(snapshot, server.id),
    );
    const own = observations.filter(
      (observation) => observation.scope === server.id,
    );
    const messages: string[] = [];
    // Use scheduler state to report whether automatic retry is paused. Fatal
    // catalog errors pause; metadata errors and agent disconnects remain scheduled.
    let paused = Boolean(attempt?.paused);
    const add = (message: string): void => {
      const trimmed = message.trim();
      if (trimmed && !messages.includes(trimmed)) messages.push(trimmed);
    };
    diagnostics.push(`${name}: ${freshnessLine(entry)}`);
    if (entry?.error) add(entry.error.message);
    if (attempt?.error && !entry?.error)
      add(normalizeCommandError(attempt.error).message);
    for (const observation of own) {
      if (observation.kind === 'catalog' || !observation.snapshot.error)
        continue;
      const error = normalizeCommandError(observation.snapshot.error);
      add(error.message);
      paused = paused || Boolean(observation.snapshot.paused);
      diagnostics.push(
        `${observationLabel(observation.kind)} on ${name}: ${error.message}`,
      );
    }
    const jobs = jobSummaries(own);
    const device = devices?.find((status) => status.profile === server.id);
    if (device) {
      const state: SyncState = device.refreshing
        ? 'refreshing'
        : device.failed
          ? 'failed'
          : device.total > 0 &&
              device.ready === device.total &&
              !device.unavailable
            ? 'ok'
            : 'unknown';
      const detail = device.refreshing
        ? `Loading device lists · ${device.ready} of ${device.total} accounts`
        : device.failed
          ? `${device.error ?? 'Some device lists are unavailable'}${device.lastSuccessAt === undefined ? '' : ` · Last updated ${clockTime(device.lastSuccessAt)}`}`
          : device.unavailable
            ? `Device access unavailable for ${device.unavailable} of ${device.total} accounts`
            : device.total
              ? `Updated ${clockTime(device.lastSuccessAt)}`
              : 'No accounts';
      jobs.push({
        kind: 'devices',
        label: 'Devices',
        state,
        detail,
        lastSuccessAt: device.lastSuccessAt,
        paused: false,
      });
      diagnostics.push(`Devices on ${name}: ${detail}`);
    }
    if (inbox) {
      const teams = snapshot.stores.filter(
        (store) => store.server === server.id && chatAvailable(snapshot, store),
      );
      let loading = false;
      let unavailable = false;
      for (const team of teams) {
        const entry = inbox.get(team.id);
        if (
          !entry ||
          (entry.state === 'loading' && !entry.data && !entry.error)
        )
          loading = true;
        else if (
          !entry.data ||
          entry.state === 'blocked' ||
          entry.state === 'unavailable' ||
          entry.stale ||
          entry.error ||
          entry.data.degraded
        )
          unavailable = true;
      }
      // Initial unread loading belongs here. Background polling with usable
      // data must not keep the global refresh button spinning.
      const chat: SyncJobSummary = {
        kind: 'chat',
        label: 'Chat',
        state: loading
          ? 'refreshing'
          : unavailable
            ? 'failed'
            : teams.length
              ? 'ok'
              : 'unknown',
        detail: loading
          ? 'Loading unread counts'
          : unavailable
            ? 'Unread counts may be incomplete or unavailable'
            : teams.length
              ? 'Unread counts loaded'
              : 'No available chats',
        paused: false,
      };
      const discovery = jobs.findIndex((job) => job.kind === 'discovery');
      jobs.splice(discovery < 0 ? jobs.length : discovery + 1, 0, chat);
    }
    const refreshing = jobs.some((job) => job.state === 'refreshing');
    const failed = messages.length > 0;
    const chatFailed = jobs.some(
      (job) => job.kind === 'chat' && job.state === 'failed',
    );
    const devicesFailed = device?.failed ?? false;
    const state: SyncState =
      failed || chatFailed || devicesFailed
        ? 'failed'
        : refreshing
          ? 'refreshing'
          : entry?.lastSuccessAt !== undefined
            ? 'ok'
            : 'unknown';
    const canReconnect =
      service.supportsConnectivity &&
      (server.host_id !== null ||
        snapshot.stores.some((store) => store.server === server.id));
    return {
      id: server.id,
      name,
      state,
      message: failed
        ? failureSentence(messages, entry?.lastSuccessAt, paused)
        : devicesFailed
          ? 'Device lists could not all be refreshed. Use Retry devices to try again.'
          : chatFailed
            ? 'Chat unread counts may be incomplete or unavailable.'
            : refreshing
              ? 'Refreshing…'
              : entry?.lastSuccessAt !== undefined
                ? `Up to date, ${timeOf(entry.lastSuccessAt)}`
                : 'No refresh observation yet',
      lastSuccessAt: entry?.lastSuccessAt,
      canReconnect: canReconnect && ((!chatFailed && !devicesFailed) || failed),
      reconnectDisabled: reconnectDisabledFor(server),
      reconnecting: Boolean(connectivity?.refreshing),
      jobs,
    };
  });
  // The registry and metadata jobs belong to no server. They are listed only
  // when one of them has failed: a healthy Mac has nothing to say here.
  const local = observations.filter(
    (observation) =>
      observation.scope === null && observation.snapshot.error !== undefined,
  );
  const localJobs = observations.filter(
    (observation) => observation.scope === null,
  );
  if (local.length || catalogAttempt?.error) {
    const messages: string[] = catalogAttempt?.error
      ? [catalogAttempt.error.message]
      : [];
    let paused = false;
    for (const observation of local) {
      const error = normalizeCommandError(observation.snapshot.error);
      if (!messages.includes(error.message)) messages.push(error.message);
      paused = paused || Boolean(observation.snapshot.paused);
      diagnostics.push(
        `${observationLabel(observation.kind)}: ${error.message}`,
      );
    }
    servers.push({
      id: null,
      name: local.length ? 'This Mac' : 'Catalog',
      state: 'failed',
      message: catalogAttempt?.error
        ? `${messages.join('; ').replace(/\.+$/, '')}. The catalog refresh did not complete.${snapshot.stores.length ? ' Previously loaded data is retained.' : ''} Use Refresh to retry.`
        : failureSentence(messages, undefined, paused).replace(
            ' No successful refresh yet.',
            '',
          ),
      canReconnect: false,
      reconnectDisabled: true,
      reconnecting: false,
      jobs: jobSummaries(localJobs),
    });
  }
  const activities: { id: string; label: string; startedAt?: number }[] =
    service.activities.getSnapshot().map((activity) => ({
      id: `catalog:${activity.id}`,
      label: `${activity.isCurrent() ? '' : 'Finishing earlier refresh · '}${activity.phases.join(' · ')}`,
      startedAt: activity.startedAt,
    }));
  // Local failures already expose all of their jobs in the This Mac group.
  // Otherwise healthy local work used to be completely invisible.
  for (const observation of observations) {
    if (!observation.snapshot.refreshing) continue;
    const represented = servers.find(
      (server) => server.id === observation.scope,
    );
    if (represented) {
      diagnostics.push(
        `Active: ${observationLabel(observation.kind)} on ${represented.name}`,
      );
      continue;
    }
    activities.push({
      id: `job:${observation.key}`,
      label: `Refreshing ${observationLabel(observation.kind).toLowerCase()}${observation.scope ? ` on ${observation.scope}` : ''}`,
      startedAt: observation.snapshot.lastAttemptAt,
    });
  }
  for (const activity of activities)
    diagnostics.push(
      `Active: ${activity.label}${activity.startedAt === undefined ? '' : ` · ${Math.max(0, Math.floor((Date.now() - activity.startedAt) / 1000))}s`} [${activity.id}]`,
    );
  return {
    refreshing:
      activities.length > 0 ||
      servers.some((server) =>
        server.jobs.some((job) => job.state === 'refreshing'),
      ),
    activities,
    failed: servers.some((server) => server.state === 'failed'),
    servers,
    diagnostics,
  };
}

/**
 * The text Copy diagnostics puts on the clipboard: the status lines first,
 * as before, then the timing log. `external` is what the other layers had
 * reported when the popover opened; the renderer's own events are read now.
 */
export function diagnosticsText(
  summary: SyncSummary,
  snapshot: AgentSnapshot,
  external: readonly TimingEvent[] = [],
  now: number = Date.now(),
): string {
  const header = [
    `agent ${snapshot.agent.state} · ${snapshot.servers.length} servers · ${
      snapshot.stores.filter((store) => store.kind === 'team').length
    } teams · window ${
      typeof document === 'undefined' ? 'unknown' : document.visibilityState
    }`,
  ];
  const events = [...diagnosticLog.events(), ...external].sort(
    (a, b) => a.at - b.at,
  );
  return [
    ...summary.diagnostics,
    '',
    formatTimings(events, { now, header }),
  ].join('\n');
}

/** The summary, re-read whenever the scheduler publishes. */
export function useSyncSummary(
  snapshot: AgentSnapshot,
  service: DesktopReconciliation,
): SyncSummary {
  useSyncExternalStore(
    service.scheduler.subscribe,
    service.scheduler.getSnapshot,
    service.scheduler.getSnapshot,
  );
  const activities = useSyncExternalStore(
    service.activities.subscribe,
    service.activities.getSnapshot,
    service.activities.getSnapshot,
  );
  // Also refresh elapsed times and superseded labels while real work remains.
  const [, tick] = useState(0);
  const busy =
    activities.length > 0 ||
    service.scheduler.observations().some((entry) => entry.snapshot.refreshing);
  useEffect(() => {
    if (!busy) return;
    const timer = setInterval(() => tick((value) => value + 1), 1_000);
    return () => clearInterval(timer);
  }, [busy]);
  const inbox = useSidebarInbox();
  const devices = useShellDeviceMetadata();
  const deviceStatuses = useSyncExternalStore(
    devices?.subscribe ?? noDeviceSubscription,
    devices?.getSnapshot ?? emptyDeviceMetadata,
    devices?.getSnapshot ?? emptyDeviceMetadata,
  );
  return summarizeSync(snapshot, service, inbox, deviceStatuses);
}

/** Names the popover as the refresh button's description while it is up. */
export const SYNC_STATUS_ID = 'sync-status-popover';

/**
 * Compact timing or retry text shown at the right of a job row. Exported so
 * the row's own text is held by a test without standing the popover up.
 */
export function JobTimes({ job }: { job: SyncJobSummary }): ReactNode {
  if (job.kind === 'devices')
    return (
      <span
        className={`sync-when sync-device-status${job.state === 'failed' ? ' failed' : ''}`}
        role={job.state === 'refreshing' ? 'status' : undefined}
      >
        {job.detail}
      </span>
    );
  if (job.kind === 'chat')
    return (
      <span
        className={`sync-when${job.state === 'failed' ? ' failed' : ''}`}
        role={job.state === 'refreshing' ? 'status' : undefined}
      >
        {job.state === 'refreshing'
          ? 'Loading unread counts…'
          : job.state === 'failed'
            ? 'Unread counts incomplete'
            : job.state === 'ok'
              ? 'Unread counts loaded'
              : 'No available chats'}
      </span>
    );
  // How long the last run took, beside the time it ran: a job that is slow
  // and a job that is stale read the same on a row that states only a time.
  const took = runDuration(job.lastMilliseconds);
  if (job.state === 'failed')
    return (
      <span className="sync-when failed">
        {job.paused
          ? 'Paused'
          : job.nextAttemptAt !== undefined
            ? `Retry at ${clockTime(job.nextAttemptAt)}`
            : ''}
      </span>
    );
  if (job.state === 'refreshing')
    return <span className="sync-when">{clockTime(job.lastSuccessAt)}</span>;
  if (job.state === 'unknown')
    return (
      <span className="sync-when">
        {job.lastAttemptAt !== undefined
          ? `Attempted ${clockTime(job.lastAttemptAt)}${took ? ` in ${took}` : ''}`
          : 'Not yet run'}
      </span>
    );
  return (
    <span className="sync-when">
      {clockTime(job.lastSuccessAt)}
      {took ? ` in ${took}` : ''}
      {job.nextAttemptAt !== undefined ? (
        <span className="sync-next">
          Next at {clockTime(job.nextAttemptAt)}
        </span>
      ) : null}
    </span>
  );
}

/** Expanded job status and recovery actions for one server. */
function ServerBody({
  server,
  service,
  onClose,
  onOpenServers,
}: {
  server: SyncServerSummary;
  service: DesktopReconciliation;
  onClose: () => void;
  onOpenServers?: (profile: string) => void;
}): ReactNode {
  const devices = useShellDeviceMetadata();
  return (
    <>
      {server.state === 'failed' ? (
        <p className="sync-cause">{server.message}</p>
      ) : null}
      {server.jobs.length ? (
        <ul className="sync-jobs" aria-label={`${server.name} jobs`}>
          {server.jobs.map((job) => (
            <li key={job.kind} title={job.detail}>
              <span className={`sync-dot ${job.state}`} aria-hidden="true" />
              <b>{job.label}</b>
              <JobTimes job={job} />
            </li>
          ))}
        </ul>
      ) : null}
      {server.state === 'failed' && server.id ? (
        <div className="sync-actions">
          {devices &&
          server.jobs.some(
            (job) => job.kind === 'devices' && job.state === 'failed',
          ) ? (
            <Button
              size="sm"
              onClick={() => devices.retry(server.id!)}
              aria-label={`Retry devices on ${server.name}`}
            >
              Retry devices
            </Button>
          ) : null}
          {server.canReconnect ? (
            <Button
              size="sm"
              disabled={server.reconnectDisabled || server.reconnecting}
              aria-label={`Reconnect ${server.name}`}
              onClick={() => service.reconnect(server.id!)}
            >
              {server.reconnecting ? 'Reconnecting' : 'Reconnect'}
            </Button>
          ) : null}
          {onOpenServers ? (
            <Button
              size="sm"
              variant="plain"
              className="lnk"
              onClick={() => {
                onClose();
                onOpenServers(server.id!);
              }}
            >
              Open server settings
            </Button>
          ) : null}
        </div>
      ) : null}
    </>
  );
}

/** Collapsible status row used when the popover contains several servers. */
function ServerRow({
  server,
  open,
  onToggle,
  ...body
}: {
  server: SyncServerSummary;
  open: boolean;
  onToggle: () => void;
  service: DesktopReconciliation;
  onClose: () => void;
  onOpenServers?: (profile: string) => void;
}): ReactNode {
  const bodyId = useId();
  const status =
    server.state === 'failed'
      ? server.message.split('. ')[0].replace(/\.$/, '')
      : server.lastSuccessAt !== undefined
        ? timeOf(server.lastSuccessAt)
        : 'Not yet refreshed';
  return (
    <div className={`sync-row${open ? ' open' : ''}`}>
      <button
        type="button"
        className="sync-head"
        aria-expanded={open}
        aria-controls={bodyId}
        onClick={onToggle}
      >
        <span className={`sync-dot ${server.state}`} aria-hidden="true" />
        <b>{server.name}</b>
        <span className={`sync-status ${server.state}`}>{status}</span>
        <Icon name="chev" className="sync-chev" />
      </button>
      <div id={bodyId} className="sync-body" hidden={!open}>
        <ServerBody server={server} {...body} />
      </div>
    </div>
  );
}

export function SyncPopover({
  snapshot,
  service,
  summary,
  anchorRef,
  onClose,
  onPointerEnter,
  onPointerLeave,
  onOpenServers,
  onRefresh,
  refreshDisabled = false,
}: {
  snapshot: AgentSnapshot;
  service: DesktopReconciliation;
  summary: SyncSummary;
  anchorRef: RefObject<HTMLElement | null>;
  onClose: () => void;
  /** Keeps a hover-held popover up while the pointer is over it. */
  onPointerEnter?: () => void;
  onPointerLeave?: () => void;
  /** Opens one server under Settings › Account, when the shell can navigate. */
  onOpenServers?: (profile: string) => void;
  /** Runs the same manual refresh action as the top-bar button. */
  onRefresh?: () => void;
  refreshDisabled?: boolean;
}): ReactNode {
  // Explicit choices override the default of expanding failed servers.
  const [openOverrides, setOpenOverrides] = useState<Record<string, boolean>>(
    {},
  );
  // The other layers' events are read while the popover is up, so the copy
  // itself stays synchronous with the click that asked for it.
  const [external, setExternal] = useState<readonly TimingEvent[]>([]);
  useEffect(() => {
    let current = true;
    diagnosticLog
      .collect()
      .then((events) => {
        if (current) setExternal(events.filter((e) => e.layer !== 'renderer'));
      })
      .catch(() => undefined);
    return () => {
      current = false;
    };
  }, []);
  const [copied, setCopied] = useState(false);
  const copiedTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  useEffect(
    () => () => {
      if (copiedTimer.current !== undefined) clearTimeout(copiedTimer.current);
    },
    [],
  );
  // Start the clipboard write during the click so WebKit retains the user
  // gesture. A promise-backed item can collect fresh diagnostics afterward;
  // older clipboard implementations use the events collected on opening.
  // Only a write the clipboard accepted says Copied.
  const copy = (): void => {
    const clipboard = navigator.clipboard;
    if (!clipboard) return;
    let writing: Promise<void>;
    try {
      if (typeof ClipboardItem !== 'undefined' && clipboard.write) {
        const content = diagnosticLog
          .collect()
          .then((events) => events.filter((e) => e.layer !== 'renderer'))
          .catch(() => external)
          .then(
            (latest) =>
              new Blob([diagnosticsText(summary, snapshot, latest)], {
                type: 'text/plain',
              }),
          );
        writing = clipboard.write([
          new ClipboardItem({ 'text/plain': content }),
        ]);
      } else {
        writing = clipboard.writeText(
          diagnosticsText(summary, snapshot, external),
        );
      }
    } catch {
      return;
    }
    void writing.then(
      () => {
        if (copiedTimer.current !== undefined)
          clearTimeout(copiedTimer.current);
        setCopied(true);
        copiedTimer.current = setTimeout(() => setCopied(false), 1_000);
      },
      () => undefined,
    );
  };
  const body = { service, onClose, onOpenServers };
  const single = summary.servers.length === 1 ? summary.servers[0] : null;
  return (
    <Popover
      anchorRef={anchorRef}
      className={`menu-portal sync-popover${single ? '' : ' multi'}`}
      align="end"
      minWidth={340}
      onClose={onClose}
      onPointerEnter={onPointerEnter}
      onPointerLeave={onPointerLeave}
    >
      <div id={SYNC_STATUS_ID} role="group" aria-label="Refresh status">
        {single ? (
          // The job rows provide the status, so a lone server needs only a
          // heading above its always-visible details.
          <div className="sync-row open">
            <div className="sync-head">
              <b>{single.name}</b>
            </div>
            <div className="sync-body">
              <ServerBody server={single} {...body} />
            </div>
          </div>
        ) : summary.servers.length ? (
          summary.servers.map((server) => {
            const key = server.id ?? 'local';
            const open =
              openOverrides[key] ??
              (server.state === 'failed' ||
                server.jobs.some((job) => job.state === 'refreshing'));
            return (
              <ServerRow
                key={key}
                server={server}
                open={open}
                onToggle={() =>
                  setOpenOverrides((prior) => ({ ...prior, [key]: !open }))
                }
                {...body}
              />
            );
          })
        ) : (
          <p className="sync-empty">
            {snapshot.servers.length
              ? 'No refresh observation yet.'
              : 'No servers configured.'}
          </p>
        )}
        {summary.activities.length ? (
          <ul
            className="sync-jobs sync-activities"
            aria-label="Other refresh activity"
          >
            {summary.activities.map((activity) => (
              <li key={activity.id}>
                <span className="sync-dot refreshing" aria-hidden="true" />
                <b>{activity.label}</b>
                <span className="sync-when">
                  {activity.startedAt === undefined
                    ? 'Running now'
                    : `${Math.max(0, Math.floor((Date.now() - activity.startedAt) / 1000))}s`}
                </span>
              </li>
            ))}
          </ul>
        ) : null}
        <div className="sync-foot">
          {onRefresh ? (
            <Button
              size="sm"
              variant="plain"
              className="lnk"
              disabled={refreshDisabled}
              onClick={onRefresh}
            >
              Refresh now
            </Button>
          ) : null}
          <Button
            size="sm"
            variant="plain"
            className="lnk"
            icon={copied ? 'check' : undefined}
            onClick={copy}
          >
            {copied ? 'Copied' : 'Copy diagnostics'}
          </Button>
        </div>
      </div>
    </Popover>
  );
}
