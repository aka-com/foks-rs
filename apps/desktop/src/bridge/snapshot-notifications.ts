import {
  PROTOCOL_CAPABILITIES,
  serverDisplayName,
  serverFactAvailability,
} from '../model';
import type {
  AvailabilityReason,
  Notification,
  ProtocolCapability,
  Server,
  ServerRestriction,
} from '../model';
import { isHardStateSchemaFailure, type CommandError } from './errors';
import type { CatalogDto } from './vault-catalog';

export function restrictionFromError(
  error: CommandError,
): ServerRestriction | undefined {
  if (error.code === 'unsupported-schema')
    return { kind: 'schema-incompatible', error };
  if (error.code === 'import-verification-required')
    return { kind: 'import-verification-required', error };
  const capability = error.details?.capability as
    ProtocolCapability | undefined;
  if (
    ['capability-denied', 'capability-unavailable'].includes(error.code) &&
    capability &&
    PROTOCOL_CAPABILITIES.includes(capability)
  )
    return { kind: 'capability-denied', capability, error };
  return undefined;
}

export function schemaInstruction(profile: string): string {
  return `Inspect Settings → Servers → ${profile} and use a client that supports this schema. FOKS will not reset the data automatically.`;
}

export function notificationsOf(
  catalog: CatalogDto,
  servers: readonly Server[],
  nowSeconds: number,
): Notification[] {
  // Generate a notification for any server state that hides stores,
  // including unverified servers.
  const stopped: Partial<
    Record<AvailabilityReason, { detail: string; action: string }>
  > = {
    'check-in-expired': {
      detail: `The signed server check-in has expired. Vaults are unavailable until a newer check-in is verified.`,
      action: 'Check in',
    },
    'check-in-unavailable': {
      detail: `No usable signed check-in is available. Vaults are unavailable until one is verified.`,
      action: 'Inspect',
    },
    'server-status-unavailable': {
      detail: `Server status is unavailable. Vaults remain unavailable until signed status can be read.`,
      action: 'Inspect',
    },
    'verification-failed': {
      detail: `Server verification failed because its history does not match the pinned record. View server details to review the error.`,
      action: 'Inspect',
    },
    'verification-required': {
      detail: `This server has not been verified yet. Check the server to establish a connection and view its contents.`,
      action: 'Verify',
    },
  };
  const notes: Notification[] = servers.flatMap((server) => {
    const availability = serverFactAvailability(server, [], { nowSeconds });
    const copy = availability.available
      ? undefined
      : stopped[availability.reason];
    return copy
      ? [
          {
            id: `${availability.available ? 'available' : availability.reason}-${server.id}`,
            profile: server.id,
            severity: 'crit' as const,
            title: `${serverDisplayName(server)} is locked`,
            ...copy,
          },
        ]
      : [];
  });
  for (const [index, failure] of catalog.failures.entries()) {
    notes.push({
      id: `catalog-${failure.profile}-${failure.scope}-${index}`,
      profile: failure.profile,
      severity: failure.error.fatal ? 'crit' : 'warn',
      title:
        failure.scope === 'store'
          ? `Could not load vault on ${failure.profile}`
          : `Could not load ${failure.source} on ${failure.profile}`,
      detail: isHardStateSchemaFailure(failure.error)
        ? `${failure.error.message} ${schemaInstruction(failure.profile)}`
        : failure.error.message,
      action: failure.error.retryable ? 'Retry' : 'Inspect',
    });
  }
  // One failed store operation can surface through several catalog scopes.
  // Collapse only exact duplicate notices; distinct causes and recovery
  // actions remain visible independently.
  const seen = new Set<string>();
  return notes.filter((note) => {
    const key = JSON.stringify([
      note.profile,
      note.severity,
      note.title,
      note.detail,
      note.action,
    ]);
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}
