/**
 * Refresh status, behind the top bar's refresh button.
 *
 * The scheduler observes several jobs per server (the catalog read, the
 * connectivity probe, team discovery) and two for this Mac (the registry and
 * account metadata). One root cause — a missing keystore record, an
 * unreachable host — fails all of them with the same message, so the
 * observations are grouped by server and deduplicated by message before they
 * are shown: one row per server, one sentence, one action. The raw
 * per-observation lines remain available through Copy diagnostics.
 */

import { useSyncExternalStore } from 'react';
import type { ReactNode, RefObject } from 'react';
import { Popover } from '/kit/overlay-primitives';
import type { AgentSnapshot, CatalogFreshnessEntry, Server } from '../model';
import { serverDisplayName } from '../model';
import type { DesktopReconciliation } from '../desktop-reconciliation';
import {
  profileRefreshKey,
  profileConnectivityKey,
} from '../desktop-reconciliation';
import { Button } from '../components';
import { connectionSecurityFailure } from '../profile-connectivity';
import { normalizeCommandError } from '../bridge';

export type SyncState = 'ok' | 'refreshing' | 'failed' | 'unknown';

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
        ? 'Connectivity reconciliation'
        : kind === 'registry'
          ? 'Profile inventory'
          : 'Catalog';
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

/** The old disclosure's per-server line, kept for Copy diagnostics. */
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
    };
  });
  // The registry and metadata jobs belong to no server. They are listed only
  // when one of them has failed: a healthy Mac has nothing to say here.
  const local = observations.filter(
    (observation) =>
      observation.scope === null && observation.snapshot.error !== undefined,
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

export function SyncPopover({
  snapshot,
  service,
  summary,
  anchorRef,
  onClose,
  onOpenServers,
}: {
  snapshot: AgentSnapshot;
  service: DesktopReconciliation;
  summary: SyncSummary;
  anchorRef: RefObject<HTMLElement | null>;
  onClose: () => void;
  /** Opens Settings › Servers on one server, when the shell can navigate. */
  onOpenServers?: (profile: string) => void;
}): ReactNode {
  const copy = (): void => {
    const text = summary.diagnostics.join('\n');
    void navigator.clipboard?.writeText(text).catch(() => undefined);
  };
  return (
    <Popover
      anchorRef={anchorRef}
      className="sync-popover"
      align="end"
      minWidth={340}
      onClose={onClose}
    >
      <div role="group" aria-label="Refresh status">
        {summary.servers.length ? (
          summary.servers.map((server) => (
            <div className="sync-row" key={server.id ?? 'local'}>
              <span className={`sync-dot ${server.state}`} aria-hidden="true" />
              <div className="sync-body">
                <b>
                  {server.name}
                  {server.state === 'failed' ? ' could not be refreshed' : ''}
                </b>
                <p>{server.message}</p>
                {server.state === 'failed' && server.id ? (
                  <div className="sync-actions">
                    {server.canReconnect ? (
                      <Button
                        size="sm"
                        disabled={
                          server.reconnectDisabled || server.reconnecting
                        }
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
              </div>
            </div>
          ))
        ) : (
          <p className="sync-empty">
            {snapshot.servers.length
              ? 'No refresh observation yet.'
              : 'No servers configured.'}
          </p>
        )}
        <div className="sync-foot">
          <Button size="sm" variant="plain" className="lnk" onClick={copy}>
            Copy diagnostics
          </Button>
        </div>
      </div>
    </Popover>
  );
}
