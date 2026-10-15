/**
 * The wire form of values the mock bridge keeps in their decoded form, for
 * the commands that answer through the mock host. Each is the inverse of the
 * bridge decoder named beside it.
 */

import type { ServerStatusSnapshot } from '../bridge/servers';
import type { Account } from '../model/types';

/** Inverse of `decodeAccount` in `bridge/accounts-groups.ts`. */
export function accountWire(account: Account): Record<string, unknown> {
  return {
    store: account.store,
    profile: account.server,
    alias: account.alias,
    username: account.username,
    ...(account.localAlias === undefined
      ? {}
      : { local_alias: account.localAlias }),
  };
}

/** Inverse of `decodeCompatibility` in `bridge/servers.ts`. */
function compatibilityWire(
  compatibility: ServerStatusSnapshot['compatibility'],
): Record<string, unknown> {
  switch (compatibility.status) {
    case 'not-required':
      return { status: 'not-required' };
    case 'required-unavailable':
      return { status: 'missing' };
    case 'required':
      return {
        status: 'validated',
        expires_at: compatibility.expiresAt,
        capabilities: [...compatibility.capabilities],
      };
    case 'incompatible':
      return {
        status: 'incompatible',
        expires_at: compatibility.expiresAt,
        reason: compatibility.reason,
      };
  }
}

/** Inverse of `decodeServerStatus` in `bridge/servers.ts`. */
export function serverStatusWire(
  status: ServerStatusSnapshot,
): Record<string, unknown> {
  return {
    profile: status.profile,
    configuredEndpoint: status.configuredEndpoint,
    host: status.host,
    compatibility: compatibilityWire(status.compatibility),
    chatSupported: status.chatSupported,
  };
}
