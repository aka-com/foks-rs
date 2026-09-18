import type { CompatibilityLease, ProtocolCapability, Server } from '../model';
import { PROTOCOL_CAPABILITIES } from '../model/types';
import {
  canonicalProbeEndpoint,
  type ProfileReconciliation,
} from '../profile-connectivity';
import { decodeCommandError, type CommandError } from './errors';
import {
  array,
  bool,
  entityId,
  integer,
  nullable,
  nullableString,
  record,
  string,
} from './validation';

export interface CheckedProfileResponse {
  profile: string;
  /** Indicates whether the server identity was newly added, updated, or unchanged. */
  acceptance: 'inserted' | 'advanced' | 'unchanged';
  lookupName: string;
  canonicalName: string;
  hostId: string;
  chain: number;
  epoch: number;
}

export interface StoredHost {
  lookupName: string;
  canonicalName: string;
  hostId: string;
  chain: number;
  epoch: number;
}

export interface ServerStatusSnapshot {
  profile: string;
  configuredProbe: string;
  host: StoredHost | null;
  /** Whether this server requires a compatibility session lease. */
  leaseRequired: boolean;
  /** Expiration timestamp of the signed lease in Unix seconds, or null if unleased. */
  leaseExpiresAt: number | null;
  compatibility: Exclude<CompatibilityLease, { status: 'requirement-unknown' }>;
  chatSupported: boolean | null;
}

export interface ServerLabelResponse {
  profile: string;
  label: string | null;
  changed: boolean;
}

/** The server's advertised client-version range and this client's verdict. */
export interface ServerVersionInfo {
  minimum: string | null;
  newest: string | null;
  message: string;
  compatible: boolean;
}

export interface CheckedServer extends StoredHost {
  profile: string;
  acceptance: 'inserted' | 'advanced' | 'unchanged';
  serverVersion: ServerVersionInfo | null;
}

function decodeServer(value: unknown, at: string): Server {
  const item = record(value, at);
  if (
    Object.keys(item).some(
      (key) =>
        !['id', 'name', 'label', 'configured_probe', 'accounts'].includes(key),
    )
  )
    throw new Error(`${at} contains unexpected server metadata fields`);
  return {
    id: string(item.id, `${at}.id`),
    name: string(item.name, `${at}.name`),
    label: nullableString(item.label, at + '.label'),
    configuredProbe: string(item.configured_probe, at + '.configured_probe'),
    host_id: null,
    chain: null,
    epoch: null,
    accounts: array(item.accounts, `${at}.accounts`, string),
    trust: { status: 'unknown' },
    compatibility: {
      status: 'requirement-unknown',
      error: {
        code: 'catalog-loading',
        message: 'Server facts have not been loaded.',
        retryable: false,
        ambiguous: false,
        fatal: false,
      },
    },
    passiveStatus: { status: 'loading' },
    connectivity: { status: 'unknown' },
    services: { chat: null },
    restrictions: [],
  };
}

export const decodeServers = (value: unknown): Server[] =>
  array(value, 'list_servers response', decodeServer);

export function decodeCheckedProfile(value: unknown): CheckedProfileResponse {
  const item = record(value, 'check_and_add_profile response');
  const acceptance = string(
    item.acceptance,
    'check_and_add_profile.acceptance',
  );
  if (
    acceptance !== 'inserted' &&
    acceptance !== 'advanced' &&
    acceptance !== 'unchanged'
  ) {
    throw new Error('check_and_add_profile.acceptance is invalid');
  }
  return {
    profile: string(item.profile, 'check_and_add_profile.profile'),
    acceptance,
    lookupName: string(item.lookupName, 'check_and_add_profile.lookupName'),
    canonicalName: string(
      item.canonicalName,
      'check_and_add_profile.canonicalName',
    ),
    hostId: entityId(item.hostId, '02', 'check_and_add_profile.hostId'),
    chain: integer(item.chain, 'check_and_add_profile.chain'),
    epoch: integer(item.epoch, 'check_and_add_profile.epoch'),
  };
}

function decodeStoredHost(value: unknown, at: string): StoredHost {
  const item = record(value, at);
  return {
    lookupName: string(item.lookupName, `${at}.lookupName`),
    canonicalName: string(item.canonicalName, `${at}.canonicalName`),
    hostId: entityId(item.hostId, '02', `${at}.hostId`),
    chain: integer(item.chain, `${at}.chain`),
    epoch: integer(item.epoch, `${at}.epoch`),
  };
}

export function decodeCompatibility(
  value: unknown,
): ServerStatusSnapshot['compatibility'] {
  const item = record(value, 'compatibility');
  const status = string(item.status, 'compatibility.status');
  if (status === 'not-required' || status === 'missing') {
    if (Object.keys(item).some((key) => key !== 'status'))
      throw new Error('Unexpected compatibility fields');
    return {
      status: status === 'missing' ? 'required-unavailable' : 'not-required',
    };
  }
  const expiresAt = integer(item.expires_at, 'compatibility.expires_at');
  if (status === 'incompatible') {
    const reason = string(item.reason, 'compatibility.reason');
    if (
      !['drift', 'protocol-mismatch', 'unknown-capability'].includes(reason) ||
      Object.keys(item).some(
        (key) => !['status', 'expires_at', 'reason'].includes(key),
      )
    )
      throw new Error('Invalid compatibility failure');
    return {
      status,
      expiresAt,
      reason: reason as 'drift' | 'protocol-mismatch' | 'unknown-capability',
    };
  }
  if (
    status !== 'validated' ||
    Object.keys(item).some(
      (key) => !['status', 'expires_at', 'capabilities'].includes(key),
    )
  )
    throw new Error('Invalid compatibility status');
  const capabilities = array(
    item.capabilities,
    'compatibility.capabilities',
    string,
  );
  if (
    !capabilities.length ||
    new Set(capabilities).size !== capabilities.length ||
    capabilities.some(
      (capability) =>
        !PROTOCOL_CAPABILITIES.includes(capability as ProtocolCapability),
    )
  )
    throw new Error('Invalid compatibility capabilities');
  return {
    status: 'required',
    expiresAt,
    capabilities: capabilities as ProtocolCapability[],
  };
}

function decodeConnectionResult<S extends string>(
  value: unknown,
  expectedProfile: string,
  allowed: readonly S[],
  fields: readonly string[] = [],
): { status: S } | { status: 'failed'; error: CommandError } {
  const item = record(value, 'connectivity observation');
  const status = string(item.status, 'connectivity observation.status');
  if (status === 'failed') {
    if (Object.keys(item).some((key) => key !== 'status' && key !== 'error'))
      throw new Error('Unexpected connectivity failure fields.');
    const error = decodeCommandError(
      item.error,
      'connectivity observation.error',
    );
    if (error.details?.profile !== expectedProfile)
      throw new Error('Connectivity error belongs to a different profile.');
    return { status: 'failed', error };
  }
  if (
    Object.keys(item).length !== fields.length + 1 ||
    Object.keys(item).some((key) => key !== 'status' && !fields.includes(key))
  )
    throw new Error(
      'Successful connectivity observation contains unexpected fields.',
    );
  for (const candidate of allowed)
    if (candidate === status) return { status: candidate };
  throw new Error('Invalid connectivity observation status.');
}

export function decodeProfileReconciliation(
  value: unknown,
  expectedProfile: string,
): ProfileReconciliation {
  const item = record(value, 'connectivity response');
  if (
    item.profile !== expectedProfile ||
    Object.keys(item).some(
      (key) => !['profile', 'identity', 'compatibility'].includes(key),
    )
  )
    throw new Error(
      'Connectivity response belongs to a different profile or has unexpected fields.',
    );
  const observed = decodeConnectionResult(
    item.identity,
    expectedProfile,
    ['connected'] as const,
    ['hostId', 'configuredProbe'],
  );
  const identity: ProfileReconciliation['identity'] =
    observed.status === 'failed'
      ? observed
      : (() => {
          const value = record(item.identity, 'connectivity identity');
          const hostId = entityId(
            value.hostId,
            '02',
            'connectivity identity.hostId',
          );
          const configuredProbe = string(
            value.configuredProbe,
            'connectivity identity.configuredProbe',
          );
          if (!canonicalProbeEndpoint(configuredProbe))
            throw new Error('Invalid verified probe endpoint.');
          return { status: 'connected' as const, hostId, configuredProbe };
        })();
  return {
    profile: expectedProfile,
    identity,
    compatibility: decodeConnectionResult(item.compatibility, expectedProfile, [
      'not-required',
      'renewed',
      'unchanged',
    ] as const),
  };
}

export function decodeServerStatus(value: unknown): ServerStatusSnapshot {
  const item = record(value, 'describe_server_status response');
  const compatibility = decodeCompatibility(item.compatibility);
  const status = {
    profile: string(item.profile, 'describe_server_status.profile'),
    configuredProbe: string(
      item.configuredProbe,
      'describe_server_status.configuredProbe',
    ),
    host: nullable(item.host, 'describe_server_status.host', decodeStoredHost),
    compatibility,
    leaseRequired: compatibility.status !== 'not-required',
    leaseExpiresAt:
      'expiresAt' in compatibility ? compatibility.expiresAt : null,
    chatSupported: nullable(
      item.chatSupported,
      'describe_server_status.chatSupported',
      bool,
    ),
  };
  if ((status.host !== null) !== (status.chatSupported !== null))
    throw new Error(
      'describe_server_status returned inconsistent host support',
    );
  return status;
}

function decodeServerVersion(value: unknown): ServerVersionInfo {
  const item = record(value, 'check_server.serverVersion');
  return {
    minimum: nullable(item.minimum, 'serverVersion.minimum', (entry, at) =>
      string(entry, at),
    ),
    newest: nullable(item.newest, 'serverVersion.newest', (entry, at) =>
      string(entry, at),
    ),
    message: string(item.message, 'serverVersion.message'),
    compatible: bool(item.compatible, 'serverVersion.compatible'),
  };
}

export function decodeCheckedServer(value: unknown): CheckedServer {
  const item = record(value, 'check_server response');
  const acceptance = string(item.acceptance, 'check_server.acceptance');
  if (!['inserted', 'advanced', 'unchanged'].includes(acceptance)) {
    throw new Error(
      'check_server.acceptance must be inserted, advanced, or unchanged',
    );
  }
  const version = item.serverVersion;
  return {
    profile: string(item.profile, 'check_server.profile'),
    acceptance: acceptance as CheckedServer['acceptance'],
    serverVersion:
      version === undefined || version === null
        ? null
        : decodeServerVersion(version),
    ...decodeStoredHost(item, 'check_server response'),
  };
}

export const decodeAddedServer = (
  value: unknown,
): { profile: string; configuredProbe: string } => {
  const item = record(value, 'add_server response');
  return {
    profile: string(item.profile, 'add_server.profile'),
    configuredProbe: string(item.configuredProbe, 'add_server.configuredProbe'),
  };
};

export const decodeServerLabelResponse = (
  value: unknown,
): ServerLabelResponse => {
  const item = record(value, 'set_server_label response');
  const keys = Object.keys(item).sort();
  if (keys.join(',') !== 'changed,label,profile') {
    throw new Error('set_server_label response has an invalid shape');
  }
  return {
    profile: string(item.profile, 'set_server_label.profile'),
    label: nullable(item.label, 'set_server_label.label', (entry, at) =>
      string(entry, at),
    ),
    changed: bool(item.changed, 'set_server_label.changed'),
  };
};

export const decodeForgottenServer = (
  value: unknown,
): { profile: string; removed: true } => {
  const item = record(value, 'forget_server response');
  if (item.removed !== true)
    throw new Error('forget_server.removed must be true');
  return {
    profile: string(item.profile, 'forget_server.profile'),
    removed: true,
  };
};
