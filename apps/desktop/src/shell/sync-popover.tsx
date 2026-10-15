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
 * observations. The summary itself is built in `sync-summary.ts`.
 */

import { useEffect, useId, useState, useSyncExternalStore } from 'react';
import type { ReactNode, RefObject } from 'react';
import { Popover } from '../../kit/overlay-primitives';
import type { AgentSnapshot } from '../model';
import { useSidebarInbox } from '../chat/inbox-provider';
import {
  useShellDeviceMetadata,
  emptyDeviceMetadata,
  noDeviceSubscription,
} from '../data/device-metadata';
import { diagnosticLog, type TimingEvent } from '../diagnostics/log';
import { useCopyDiagnostics } from '../diagnostics/use-copy-diagnostics';
import type { DesktopReconciliation } from '../data/desktop-reconciliation';
import { Button, Icon } from '../components';
import {
  clockTime,
  diagnosticsText,
  runDuration,
  summarizeSync,
  syncStatusLabel,
} from './sync-summary';
import type {
  SyncJobSummary,
  SyncServerSummary,
  SyncSummary,
} from './sync-summary';

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
  const status = (text: ReactNode) => (
    <td
      colSpan={3}
      className={`sync-when${job.kind === 'devices' ? ' sync-device-status' : ''}${job.state === 'failed' ? ' failed' : ''}`}
      role={job.state === 'refreshing' ? 'status' : undefined}
    >
      {text}
    </td>
  );
  if (job.kind === 'devices') return status(job.detail);
  if (job.kind === 'chat')
    return status(
      job.state === 'refreshing'
        ? 'Loading unread counts…'
        : job.state === 'failed'
          ? 'Unread counts incomplete'
          : job.state === 'ok'
            ? 'Unread counts loaded'
            : 'No available chats',
    );
  if (job.state === 'failed')
    return status(
      job.paused
        ? 'Paused'
        : job.nextAttemptAt !== undefined
          ? `Retry at ${clockTime(job.nextAttemptAt)}`
          : '',
    );
  if (job.state === 'unknown' && job.lastAttemptAt === undefined)
    return status('Not yet run');
  const last = job.state === 'unknown' ? job.lastAttemptAt : job.lastSuccessAt;
  return (
    <>
      <td className="sync-when sync-next">
        {job.state === 'ok' && job.nextAttemptAt !== undefined
          ? `Next ${clockTime(job.nextAttemptAt)}`
          : null}
      </td>
      <td className="sync-when sync-duration">
        {runDuration(job.lastMilliseconds)}
      </td>
      <td className="sync-when sync-last">
        {last === undefined ? null : clockTime(last)}
      </td>
    </>
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
        <table
          className="sync-jobs sync-job-table"
          aria-label={`${server.profileName} jobs`}
        >
          <tbody>
            {server.jobs.map((job) => (
              <tr key={job.kind} title={job.detail}>
                <th scope="row">
                  <span className="sync-job-name">
                    <span
                      className={`sync-dot ${job.state}`}
                      aria-hidden="true"
                    />
                    <b>{job.label}</b>
                  </span>
                </th>
                <JobTimes job={job} />
              </tr>
            ))}
          </tbody>
        </table>
      ) : null}
      {server.state === 'failed' && server.profileName ? (
        <div className="sync-actions">
          {devices &&
          server.jobs.some(
            (job) => job.kind === 'devices' && job.state === 'failed',
          ) ? (
            <Button
              size="sm"
              onClick={() => devices.retry(server.profileName!)}
              aria-label={`Retry devices on ${server.profileName}`}
            >
              Retry devices
            </Button>
          ) : null}
          {server.canReconnect ? (
            <Button
              size="sm"
              disabled={server.reconnectDisabled || server.reconnecting}
              aria-label={`Reconnect ${server.profileName}`}
              onClick={() => service.reconnect(server.profileName!)}
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
                onOpenServers(server.profileName!);
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
  const status = syncStatusLabel(server);
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
        <b>{server.displayLabel}</b>
        <span className={`sync-status ${server.state}`}>{status}</span>
        <Icon name="chevronDown" className="sync-chev" />
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
  onFocusEnter,
  onFocusLeave,
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
  onFocusEnter?: () => void;
  onFocusLeave?: () => void;
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
  // A promise-backed clipboard item collects fresh diagnostics after the
  // click; older clipboard implementations use the events collected on
  // opening.
  const { copied, copy } = useCopyDiagnostics(
    () =>
      diagnosticLog
        .collect()
        .then((events) => events.filter((e) => e.layer !== 'renderer'))
        .catch(() => external)
        .then((latest) => diagnosticsText(summary, snapshot, latest)),
    () => diagnosticsText(summary, snapshot, external),
  );
  const body = { service, onClose, onOpenServers };
  const single = summary.servers.length === 1 ? summary.servers[0] : null;
  return (
    <Popover
      anchorRef={anchorRef}
      className={`menu-portal sync-popover${single ? '' : ' multi'}`}
      align="end"
      minWidth={400}
      onClose={onClose}
      onPointerEnter={onPointerEnter}
      onPointerLeave={onPointerLeave}
    >
      <div
        id={SYNC_STATUS_ID}
        role="group"
        aria-label="Refresh status"
        onFocus={onFocusEnter}
        onBlur={onFocusLeave}
      >
        {single ? (
          // The job rows provide the status, so a lone server needs only a
          // heading above its always-visible details.
          <div className="sync-row open">
            <div className="sync-head">
              <b>{single.displayLabel}</b>
            </div>
            <div className="sync-body">
              <ServerBody server={single} {...body} />
            </div>
          </div>
        ) : summary.servers.length ? (
          summary.servers.map((server) => {
            const key = server.profileName ?? 'local';
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
              ? 'Not refreshed.'
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
          <Button
            size="sm"
            variant="plain"
            className="lnk"
            icon={copied ? 'check' : undefined}
            onClick={copy}
          >
            {copied ? 'Copied' : 'Copy diagnostics'}
          </Button>
          {onRefresh ? (
            <Button
              size="sm"
              variant="plain"
              icon="refresh"
              aria-label="Refresh now"
              title="Refresh now"
              disabled={refreshDisabled}
              onClick={onRefresh}
            >
              Refresh now
            </Button>
          ) : null}
        </div>
      </div>
    </Popover>
  );
}
