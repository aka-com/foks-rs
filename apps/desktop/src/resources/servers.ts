/**
 * A server's passive status, as one repository query per server.
 *
 * The Servers list, a server's own page and anything else that states a
 * server's signed status read the same row, loaded once on first subscribe
 * and kept for the repository's freshness window. The shell clears and
 * invalidates the repository on identity changes and forced refreshes, so a
 * row never outlives the catalog it was read beside.
 */

import type { Bridge, ServerStatusSnapshot } from '../bridge';
import type { MetadataQuery, MetadataRepository } from '../metadata-repository';
import type { Server } from '../model';
import {
  readCurrentServerStatus,
  serverBinding,
} from '../screens/servers/server-workflow';

export const serverStatusKey = (binding: string) =>
  ['server-status', binding] as const;

/**
 * The query for one server's status. `null` data is a read that answered
 * with no row, as opposed to one that has not answered yet.
 */
export function serverStatusQuery(
  repository: MetadataRepository,
  bridge: Bridge,
  server: Server,
): MetadataQuery<ServerStatusSnapshot | null> {
  return repository.query(serverStatusKey(serverBinding(server)), async () => {
    // The repository discards a result that lands after its query was
    // invalidated or retired, so the read itself has no earlier reason to
    // stop.
    const status = await readCurrentServerStatus(bridge, server, () => true);
    return status ?? null;
  });
}
