import { useSyncExternalStore } from 'react';
import type { ReactNode } from 'react';
import type { AgentSnapshot, CatalogFreshnessEntry } from '../model';
import { serverDisplayName } from '../model';
import type { DesktopReconciliation } from '../desktop-reconciliation';
import { profileRefreshKey } from '../desktop-reconciliation';
import { normalizeCommandError } from '../bridge';

function freshnessLabel(entry: CatalogFreshnessEntry | undefined): string {
  if (!entry) return 'No refresh observation yet';
  const successful =
    entry.lastSuccessAt === undefined
      ? 'No successful refresh'
      : `Last successful refresh: ${new Date(entry.lastSuccessAt * 1_000).toLocaleTimeString()}`;
  if (entry.error)
    return `${successful}. Refresh failed: ${entry.error.message}`;
  return entry.refreshing ? `${successful}. Refreshing` : successful;
}

export function SyncStatus({
  snapshot,
  service,
  storeId,
}: {
  snapshot: AgentSnapshot;
  service: DesktopReconciliation;
  storeId?: string;
}): ReactNode {
  useSyncExternalStore(
    service.scheduler.subscribe,
    service.scheduler.getSnapshot,
    service.scheduler.getSnapshot,
  );
  const entries = Object.values(snapshot.catalogFreshness?.profiles ?? {});
  const backgroundFailures = service.scheduler
    .observations()
    .filter(
      (entry) => entry.kind !== 'catalog' && entry.snapshot.error !== undefined,
    );
  const failed =
    entries.some((entry) => entry.error) || backgroundFailures.length > 0;
  const refreshing = entries.some((entry) => entry.refreshing);
  const selected = storeId
    ? snapshot.stores.find((store) => store.id === storeId)
    : undefined;
  return (
    <details className="sync-status">
      <summary>
        {failed
          ? 'Some data could not be refreshed'
          : refreshing
            ? 'Refreshing data'
            : 'Refresh status'}
      </summary>
      <ul>
        {snapshot.servers.map((server) => {
          const attempt = service.scheduler.snapshot(
            profileRefreshKey(snapshot, server.id),
          );
          return (
            <li key={server.id}>
              <strong>{serverDisplayName(server)}</strong>:{' '}
              {freshnessLabel(snapshot.catalogFreshness?.profiles[server.id])}
              {attempt?.error &&
              !snapshot.catalogFreshness?.profiles[server.id]?.error
                ? normalizeCommandError(attempt.error).fatal
                  ? '. Automatic refresh is paused; use Refresh to retry.'
                  : '. Automatic refresh will retry.'
                : ''}
            </li>
          );
        })}
        {backgroundFailures.map((entry) => {
          const error = normalizeCommandError(entry.snapshot.error);
          const server = snapshot.servers.find(
            (candidate) => candidate.id === entry.scope,
          );
          const label =
            entry.kind === 'discovery'
              ? 'Team discovery'
              : entry.kind === 'metadata'
                ? 'Account metadata'
                : 'Profile inventory';
          return (
            <li key={entry.key}>
              {label}
              {server ? ` on ${serverDisplayName(server)}` : ''}:{' '}
              {error.message}
              {error.fatal
                ? ' Use Refresh to retry.'
                : ' Will retry automatically.'}
            </li>
          );
        })}
        {selected ? (
          <li>
            <strong>{selected.name}</strong>:{' '}
            {freshnessLabel(snapshot.catalogFreshness?.stores[selected.id])}
          </li>
        ) : null}
      </ul>
    </details>
  );
}
