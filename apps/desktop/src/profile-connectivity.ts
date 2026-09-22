import type { CommandError } from './bridge';
import type { Server } from './model';

export type ProfileIdentityResult =
  | { status: 'connected'; hostId: string; configuredEndpoint: string }
  | { status: 'failed'; error: CommandError };
export type ProfileCompatibilityResult =
  | { status: 'not-required' | 'renewed' | 'unchanged' }
  | { status: 'failed'; error: CommandError };
export interface ProfileReconciliation {
  profile: string;
  identity: ProfileIdentityResult;
  compatibility: ProfileCompatibilityResult;
}
export type ProfileConnectivity =
  | { status: 'unknown' }
  | {
      status: 'observed';
      profile: string;
      configuredEndpoint: string;
      hostId: string | null;
      observedAt: number;
      identity: ProfileIdentityResult;
      compatibility: ProfileCompatibilityResult;
    };

export function canonicalProbeEndpoint(input: string): string | null {
  const address = input.trim();
  let host: string,
    port = '4430';
  if (address.startsWith('[')) {
    const match = /^\[([^\]]+)\](?::(\+?\d+))?$/.exec(address);
    if (!match) return null;
    host = `[${match[1]}]`;
    port = match[2] ?? port;
  } else if (address.split(':').length > 2) host = `[${address}]`;
  else {
    const parts = address.split(':');
    host = parts[0].replace(/\.+$/, '').toLowerCase();
    port = parts[1] ?? port;
    if (
      !host ||
      host.length > 253 ||
      !host
        .split('.')
        .every((part) => /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/.test(part))
    )
      return null;
  }
  if (!/^\+?\d+$/.test(port) || Number(port) < 1 || Number(port) > 65535)
    return null;
  if (host.startsWith('[')) {
    try {
      host = new URL(`https://${host}/`).hostname;
    } catch {
      return null;
    }
  }
  return `${host}:${Number(port)}`;
}

export function observeProfileConnection(
  server: Server,
  report: ProfileReconciliation,
  observedAt: number,
): ProfileConnectivity {
  const identity = report.identity;
  if (
    server.profileName !== report.profile ||
    (identity.status === 'connected' &&
      (!canonicalProbeEndpoint(server.configuredEndpoint) ||
        canonicalProbeEndpoint(server.configuredEndpoint) !==
          canonicalProbeEndpoint(identity.configuredEndpoint) ||
        (server.host_id !== null && server.host_id !== identity.hostId) ||
        server.trust.status === 'unprobed'))
  )
    throw Object.assign(
      new Error('Connectivity observation changed profile identity.'),
      {
        code: 'catalog-read-retired',
        retryable: false,
        fatal: false,
        ambiguous: false,
      },
    );
  return {
    status: 'observed',
    profile: server.profileName,
    configuredEndpoint: server.configuredEndpoint,
    hostId: identity.status === 'connected' ? identity.hostId : server.host_id,
    observedAt,
    identity: structuredClone(report.identity),
    compatibility: structuredClone(report.compatibility),
  };
}
/**
 * Returns whether a connectivity result matches all server identity, probe,
 * trust, and compatibility data already in the snapshot. Results that introduce
 * or change any of these values require a catalog refresh.
 */
export function connectionFactsUnchanged(
  server: Server,
  report: ProfileReconciliation,
): boolean {
  const { identity, compatibility } = report;
  if (
    server.profileName !== report.profile ||
    identity.status !== 'connected' ||
    // Only a verified server has a stored host identity that can be compared with
    // the observation.
    server.trust.status !== 'verified' ||
    server.host_id === null ||
    server.host_id !== identity.hostId
  )
    return false;
  const probe = canonicalProbeEndpoint(server.configuredEndpoint);
  if (!probe || probe !== canonicalProbeEndpoint(identity.configuredEndpoint))
    return false;
  return compatibility.status === 'not-required'
    ? server.compatibility.status === 'not-required'
    : compatibility.status === 'unchanged' &&
        server.compatibility.status === 'required';
}
export function retainProfileConnection(
  server: Server,
  observation: ProfileConnectivity | undefined,
): ProfileConnectivity {
  return observation?.status === 'observed' &&
    observation.profile === server.profileName &&
    observation.configuredEndpoint === server.configuredEndpoint &&
    (observation.hostId === server.host_id ||
      (server.host_id === null && server.trust.status === 'unknown'))
    ? observation
    : { status: 'unknown' };
}
export function connectionObservationFresh(
  observation: ProfileConnectivity,
  nowSeconds: number,
  maximumAge = 30,
): boolean {
  if (observation.status !== 'observed') return false;
  const elapsed = nowSeconds - observation.observedAt;
  return elapsed >= 0 && elapsed < maximumAge;
}
export function connectionSecurityFailure(
  observation: ProfileConnectivity,
): CommandError | undefined {
  if (
    observation.status !== 'observed' ||
    observation.identity.status !== 'failed'
  )
    return;
  const error = observation.identity.error;
  return [
    'saved-trust-missing',
    'security-state-missing',
    'rollback-detected',
    'checkpoint-reset-required',
    'server-verification-failed',
    'server-identity-rejected',
    'import-verification-required',
  ].includes(error.code)
    ? error
    : undefined;
}
