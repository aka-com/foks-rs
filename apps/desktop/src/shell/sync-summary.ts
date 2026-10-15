/**
 * The refresh status the top bar's sync popover draws, built from the
 * scheduler's observations, the catalog freshness and the device metadata
 * statuses, with no React or DOM dependency besides the document's
 * visibility in the copied diagnostics.
 *
 * Observations are grouped by server and duplicate errors are collapsed: one
 * root cause, such as a missing keystore record or an unreachable host, fails
 * every job on a server with the same message. Copy diagnostics retains the
 * ungrouped observations.
 */

import type { AgentSnapshot, CatalogFreshnessEntry, Server } from '../model';
import { chatAvailable, serverDisplayLabel } from '../model';
import type { TeamInbox } from '../chat/inbox-service';
import type { DeviceMetadataStatus } from '../data/device-metadata';
import { formatMilliseconds, formatTimings } from '../diagnostics/format';
import { diagnosticLog, type TimingEvent } from '../diagnostics/log';
import type { DesktopReconciliation } from '../data/desktop-reconciliation';
import type {
  ReconciliationKind,
  ReconciliationSnapshot,
} from '../scheduling/reconciliation';
import {
  profileRefreshKey,
  profileConnectivityKey,
} from '../data/desktop-reconciliation';
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
  /** The profile name, or `null` for observations that belong to this Mac. */
  profileName: string | null;
  displayLabel: string;
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
export function clockTime(milliseconds: number | undefined): string {
  return milliseconds === undefined ? '' : timeOf(milliseconds / 1_000);
}

/**
 * How long the job's last run took, as the row and its sentence state it.
 * Taken from the scheduler's own snapshot, so a cleared timing log does not
 * take the duration off the row with it.
 */
export function runDuration(milliseconds: number | undefined): string {
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
      ? `Next ${clockTime(snapshot.nextAttemptAt)}.`
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
  const causes = messages
    .flatMap((message) => message.split(/;\s+/))
    .map((message) => message.trim().replace(/\.+$/, ''))
    .filter(Boolean)
    .map((message) => `${message}.`);
  const shown =
    lastSuccessAt === undefined
      ? 'No successful refresh yet.'
      : `Showing data from ${timeOf(lastSuccessAt)}.`;
  const next = paused
    ? 'Automatic refresh is paused; use Refresh to retry.'
    : 'Retrying automatically.';
  const last = causes.pop() ?? '';
  return [...causes, `${last} ${shown} ${next}`.trim()].join('\n');
}

/** Keep dense device rows to the actionable first sentence of an error. */
function firstSentence(message: string): string {
  return message.trim().match(/^.*?[.!?](?=\s|$)/)?.[0] ?? message.trim();
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
  inbox?: ReadonlyMap<string, TeamInbox>,
  devices?: readonly DeviceMetadataStatus[],
): SyncSummary {
  const observations = service.scheduler.observations();
  const diagnostics: string[] = [];
  const catalogAttempt = snapshot.catalogFreshness?.attempt;
  if (catalogAttempt)
    diagnostics.push(`Catalog refresh: ${freshnessLine(catalogAttempt)}`);
  const servers: SyncServerSummary[] = snapshot.servers.map((server) => {
    const name = serverDisplayLabel(server);
    const entry = snapshot.catalogFreshness?.profiles[server.profileName];
    const attempt = service.scheduler.snapshot(
      profileRefreshKey(snapshot, server.profileName),
    );
    const connectivity = service.scheduler.snapshot(
      profileConnectivityKey(snapshot, server.profileName),
    );
    const own = observations.filter(
      (observation) => observation.scope === server.profileName,
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
    const device = devices?.find(
      (status) => status.profile === server.profileName,
    );
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
        ? `Loading device lists (${device.ready} of ${device.total})`
        : device.failed
          ? `${device.error ? firstSentence(device.error) : 'Some device lists are unavailable'}${device.lastSuccessAt === undefined ? '' : ` · Last updated ${clockTime(device.lastSuccessAt)}`}`
          : device.unavailable
            ? `Device access unavailable for ${device.unavailable} of ${device.total} accounts`
            : device.total
              ? clockTime(device.lastSuccessAt)
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
        (store) =>
          store.server === server.profileName && chatAvailable(snapshot, store),
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
      const devicesIndex = jobs.findIndex((job) => job.kind === 'devices');
      const discovery = jobs.findIndex((job) => job.kind === 'discovery');
      const after = devicesIndex >= 0 ? devicesIndex + 1 : discovery + 1;
      jobs.splice(after > 0 ? after : jobs.length, 0, chat);
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
        snapshot.stores.some((store) => store.server === server.profileName));
    return {
      profileName: server.profileName,
      displayLabel: name,
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
    const catalogCancelled =
      catalogAttempt?.error !== undefined &&
      normalizeCommandError(catalogAttempt.error).code === 'cancelled';
    servers.push({
      profileName: null,
      displayLabel: local.length ? 'This Mac' : 'Catalog',
      state: 'failed',
      message: catalogAttempt?.error
        ? catalogCancelled
          ? 'Request cancelled: The catalog refresh did not complete. Use Refresh to retry.'
          : `${messages.join('; ').replace(/\.+$/, '')}. The catalog refresh did not complete.${snapshot.stores.length ? ' Previously loaded data is retained.' : ''} Use Refresh to retry.`
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
      label: `${activity.isCurrent() ? '' : 'Finishing earlier refresh · '}${
        activity.phases.every((phase) => phase.startsWith('Loading '))
          ? `Loading ${activity.phases.map((phase) => phase.slice('Loading '.length)).join(', ')}`
          : activity.phases.join(' · ')
      }`,
      startedAt: activity.startedAt,
    }));
  // Local failures already expose all of their jobs in the This Mac group.
  // Otherwise healthy local work used to be completely invisible.
  for (const observation of observations) {
    if (!observation.snapshot.refreshing) continue;
    const represented = servers.find(
      (server) => server.profileName === observation.scope,
    );
    if (represented) {
      diagnostics.push(
        `Active: ${observationLabel(observation.kind)} on ${represented.displayLabel}`,
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

/** Compact state at the right of a refresh row; the expanded body has detail. */
export function syncStatusLabel(server: SyncServerSummary): string {
  if (server.state === 'failed')
    return /\bcancell?ed\b/i.test(server.message) ? 'Cancelled' : 'Failed';
  if (server.state === 'refreshing') return 'Refreshing';
  if (server.lastSuccessAt !== undefined) return timeOf(server.lastSuccessAt);
  return 'Not refreshed';
}
