/**
 * Refresh status, behind the top bar's refresh button.
 *
 * The scheduler observes several jobs per server (the catalog read, the
 * connectivity probe, team discovery) and two for this Mac (the registry and
 * account metadata). One root cause — a missing keystore record, an
 * unreachable host — fails all of them with the same message, so the
 * observations are grouped by server and duplicate errors are collapsed. Each
 * server lists its jobs with their most recent success and next-run times.
 * Failures add a summary and recovery actions. A single server is always
 * expanded; when several servers are present, failed servers start expanded
 * and the others start collapsed. Copy diagnostics retains the ungrouped
 * observations.
 */

import { useId, useState, useSyncExternalStore } from 'react';
import type { ReactNode, RefObject } from 'react';
import { Popover } from '/kit/overlay-primitives';
import type { AgentSnapshot, CatalogFreshnessEntry, Server } from '../model';
import { serverDisplayName } from '../model';
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
  kind: ReconciliationKind;
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
    ? 'Team discovery'
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
  const times = {
    lastSuccessAt: snapshot.lastSuccessAt,
    lastAttemptAt: snapshot.lastAttemptAt,
    nextAttemptAt: snapshot.nextAttemptAt,
    paused: Boolean(snapshot.paused),
  };
  if (snapshot.error) {
    const cause = normalizeCommandError(snapshot.error).message.replace(
      /\.+$/,
      '',
    );
    return {
      kind,
      label,
      state: 'failed',
      detail: line([`${cause}.`, succeeded, next]),
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
      detail: line([`Succeeded ${clockTime(snapshot.lastSuccessAt)}.`, next]),
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
  return entry.refreshing ? `${successful}. Refreshing` : successful;
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
    const refreshing =
      Boolean(entry?.refreshing) ||
      own.some((observation) => observation.snapshot.refreshing);
    const failed = messages.length > 0;
    const state: SyncState = failed
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
        : refreshing
          ? 'Refreshing…'
          : entry?.lastSuccessAt !== undefined
            ? `Up to date, ${timeOf(entry.lastSuccessAt)}`
            : 'No refresh observation yet',
      lastSuccessAt: entry?.lastSuccessAt,
      canReconnect,
      reconnectDisabled: reconnectDisabledFor(server),
      reconnecting: Boolean(connectivity?.refreshing),
      jobs: jobSummaries(own),
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
  return {
    refreshing:
      Boolean(catalogAttempt?.refreshing) ||
      servers.some((server) => server.state === 'refreshing'),
    failed: servers.some((server) => server.state === 'failed'),
    servers,
    diagnostics,
  };
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
  return summarizeSync(snapshot, service);
}

/** Names the popover as the refresh button's description while it is up. */
export const SYNC_STATUS_ID = 'sync-status-popover';

/** Compact timing or retry text shown at the right of a job row. */
function JobTimes({ job }: { job: SyncJobSummary }): ReactNode {
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
          ? `Attempted ${clockTime(job.lastAttemptAt)}`
          : 'Not yet run'}
      </span>
    );
  return (
    <span className="sync-when">
      {clockTime(job.lastSuccessAt)}
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
}: {
  snapshot: AgentSnapshot;
  service: DesktopReconciliation;
  summary: SyncSummary;
  anchorRef: RefObject<HTMLElement | null>;
  onClose: () => void;
  /** Keeps a hover-held popover up while the pointer is over it. */
  onPointerEnter?: () => void;
  onPointerLeave?: () => void;
  /** Opens Settings › Servers on one server, when the shell can navigate. */
  onOpenServers?: (profile: string) => void;
  /** Runs the same manual refresh action as the top-bar button. */
  onRefresh?: () => void;
}): ReactNode {
  // Explicit choices override the default of expanding failed servers.
  const [openOverrides, setOpenOverrides] = useState<Record<string, boolean>>(
    {},
  );
  const copy = (): void => {
    const text = summary.diagnostics.join('\n');
    void navigator.clipboard?.writeText(text).catch(() => undefined);
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
            const open = openOverrides[key] ?? server.state === 'failed';
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
        <div className="sync-foot">
          {onRefresh ? (
            <Button
              size="sm"
              variant="plain"
              className="lnk"
              disabled={summary.refreshing}
              onClick={onRefresh}
            >
              Refresh now
            </Button>
          ) : null}
          <Button size="sm" variant="plain" className="lnk" onClick={copy}>
            Copy diagnostics
          </Button>
        </div>
      </div>
    </Popover>
  );
}
