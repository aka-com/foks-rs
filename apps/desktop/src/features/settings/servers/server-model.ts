/**
 * The pure parts of a server's page and row: the state its mark and chip
 * show, the identity the catalog pins for it, and how its signed
 * compatibility expiry and a refused client version are written.
 */

import type {
  CheckedServer,
  ServerStatusSnapshot,
  StoredHost,
} from '../../../bridge';
import { serverAvailability } from '../../../model';
import type { AgentSnapshot, Server } from '../../../model';

/** False for a blocked server or one whose saved schema this client cannot read. */
export function canReadServer(server: Server): boolean {
  return (
    server.trust.status !== 'blocked' &&
    !server.restrictions.some((row) => row.kind === 'schema-incompatible')
  );
}

export const expires = (value: number | null): string => {
  if (value === null) return 'No expiration date';
  const date = new Date(value * 1000);
  return Number.isFinite(date.getTime())
    ? new Intl.DateTimeFormat(undefined, {
        dateStyle: 'medium',
        timeStyle: 'short',
      }).format(date)
    : 'Invalid date';
};
/** Formats expiration timestamps for compact server row displays. */
export const expiresShort = (value: number | null): string => {
  if (value === null) return 'no expiration';
  const date = new Date(value * 1000);
  return Number.isFinite(date.getTime())
    ? new Intl.DateTimeFormat(undefined, {
        month: 'short',
        day: 'numeric',
        hour: 'numeric',
        minute: '2-digit',
      }).format(date)
    : 'Unknown date';
};
/** Explains why a checked server does not accept this client's v0.1.9. */
export function versionMismatchText(
  version: NonNullable<CheckedServer['serverVersion']>,
): string {
  const bounds: string[] = [];
  if (version.minimum) bounds.push(`requires ${version.minimum} or newer`);
  if (version.newest) bounds.push(`supports up to ${version.newest}`);
  const range = bounds.length ? `; it ${bounds.join(' and ')}` : '';
  const message = version.message ? ` ${version.message}` : '';
  return `This server does not accept FOKS client 0.1.9${range}.${message}`;
}

export function serverFor(
  agentSnapshot: AgentSnapshot,
  profile: string | undefined,
): Server | undefined {
  return profile
    ? agentSnapshot.servers.find((server) => server.profileName === profile)
    : undefined;
}

/** The identity pinned for this server, as the catalog carries it. */
export type PinnedHost = Pick<StoredHost, 'hostId' | 'chain' | 'epoch'>;

export function pinnedHost(server: Server): PinnedHost | null {
  // A server whose trust is blocked, or whose saved schema this client
  // cannot read, states no identity here; the page says so instead. The
  // three fields are projected from one signed status, so either all of them
  // describe a pinned host or none of them does.
  return canReadServer(server) &&
    server.host_id !== null &&
    server.chain !== null &&
    server.epoch !== null
    ? { hostId: server.host_id, chain: server.chain, epoch: server.epoch }
    : null;
}

/**
 * When this server's signed compatibility result expires, in Unix seconds,
 * or null where the pinned protocol states no expiry. The expiry belongs to
 * the signed result itself, which the catalog carries, so it is read from
 * there; a check's own status read is newer than the catalog refresh it asks
 * for and answers until that refresh lands.
 */
export function leaseExpiry(
  server: Server,
  checked?: ServerStatusSnapshot,
): number | null {
  const lease = checked?.compatibility ?? server.compatibility;
  return 'expiresAt' in lease ? lease.expiresAt : null;
}

/** UI state representing server health and connectivity. */
export type ServerUiState =
  | 'checked'
  | 'unprobed'
  | 'lapsed'
  | 'unavailable'
  | 'blocked'
  | 'schema'
  | 'incompatible'
  | 'import-verification'
  | 'recovery-required'
  | 'pending';

export const STATE_LABEL: Readonly<Record<ServerUiState, string>> = {
  checked: 'Checked',
  unprobed: 'Never checked',
  lapsed: 'Check-in expired',
  unavailable: 'Status unknown',
  blocked: 'Untrusted',
  schema: 'Schema incompatible',
  incompatible: 'Protocol incompatible',
  'import-verification': 'Verification required',
  'recovery-required': 'Security state missing',
  pending: 'Checking status',
};

/** Returns true if the server state blocks read and write operations. */
export const isLocked = (state: ServerUiState): boolean =>
  state === 'lapsed' ||
  state === 'unavailable' ||
  state === 'blocked' ||
  state === 'schema' ||
  state === 'incompatible' ||
  state === 'import-verification' ||
  state === 'recovery-required';

export const markTone = (state: ServerUiState): string =>
  state === 'checked' ? 'ok' : isLocked(state) ? 'bad' : '';

/** The state a server's mark and chip show, from `serverAvailability`. */
export function resolveServerUiState(
  agentSnapshot: AgentSnapshot,
  server: Server,
): ServerUiState {
  const availability = serverAvailability(agentSnapshot, server);
  if (availability.available) return 'checked';
  if (availability.reason === 'loading') return 'pending';
  if (availability.reason === 'compatibility-incompatible')
    return 'incompatible';
  if (availability.reason === 'security-state-missing')
    return 'recovery-required';
  if (availability.reason === 'verification-required') return 'unprobed';
  if (availability.reason === 'check-in-expired') return 'lapsed';
  if (availability.reason === 'verification-failed') return 'blocked';
  if (availability.reason === 'schema-incompatible') return 'schema';
  if (availability.reason === 'import-verification-required')
    return 'import-verification';
  return 'unavailable';
}
