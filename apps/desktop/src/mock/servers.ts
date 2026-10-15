/**
 * The mock's server commands, the members of `bridge/commands-servers.ts`,
 * which answer from the shared server records with decoded values.
 */

import type { Bridge } from '../bridge';
import type { serverCommands } from '../bridge/commands-servers';
import { failure, HOST_IDS, type MockDomain, type MockState } from './state';

/** `describe_server_status` as the mock answers it. */
export function mockServerStatus(
  state: MockState,
): Bridge['describeServerStatus'] {
  const { servers, serverProbes, serverHosts } = state;
  return async (profile) => {
    const server = servers.find((entry) => entry.profileName === profile);
    if (!server)
      throw failure('store-not-found', 'That server is not configured.');
    const compatibility = server.compatibility;
    if (compatibility.status === 'requirement-unknown')
      throw compatibility.error;
    return {
      profile: server.profileName,
      configuredEndpoint:
        serverProbes.get(server.profileName) ?? server.profileName,
      host: serverHosts.get(server.profileName) ?? null,
      leaseRequired: compatibility.status !== 'not-required',
      leaseExpiresAt:
        'expiresAt' in compatibility ? compatibility.expiresAt : null,
      compatibility,
      chatSupported: serverHosts.has(server.profileName)
        ? server.services.chat
        : null,
    };
  };
}

export function mockServerCommands(
  state: MockState,
): MockDomain<typeof serverCommands> {
  const { servers, serverProbes, serverHosts, firstRunFixture } = state;
  return {
    // An optional method only the native host offers; callers fall back to
    // other paths when a bridge lacks it.
    reconcileServer: undefined,
    listServers: () =>
      Promise.resolve(servers.map((server) => ({ ...server }))),
    checkAndAddProfile: async (profileName, probe) => {
      const path =
        probe === firstRunFixture.invited.server
          ? firstRunFixture.invited
          : probe === firstRunFixture.own.server
            ? firstRunFixture.own
            : undefined;
      if (!path) throw failure('io', `No response from ${probe}`);
      return { ...path.report, profile: profileName };
    },
    checkAndAddGoProfile: async (_candidateId, hostId, profileName, probe) => {
      const path =
        probe === firstRunFixture.invited.server
          ? firstRunFixture.invited
          : probe === firstRunFixture.own.server
            ? firstRunFixture.own
            : undefined;
      if (!path || path.report.hostId !== hostId)
        throw failure('io', `${probe} did not match, so nothing was saved.`);
      return { ...path.report, profile: profileName };
    },
    describeServerStatus: mockServerStatus(state),
    checkServer: async (profile) => {
      const server = servers.find((entry) => entry.profileName === profile);
      if (!server)
        throw failure('store-not-found', 'That server is not configured.');
      const existing = serverHosts.get(server.profileName);
      const host = existing ?? {
        lookupName: serverProbes.get(server.profileName) ?? server.profileName,
        canonicalName:
          serverProbes.get(server.profileName) ?? server.profileName,
        hostId: HOST_IDS[server.profileName] ?? `02${'7'.repeat(64)}`,
        chain: 4,
        epoch: 118204,
      };
      serverHosts.set(server.profileName, host);
      if (!existing && server.services.chat === null)
        server.services = { chat: false };
      return {
        profile: server.profileName,
        acceptance: existing ? ('unchanged' as const) : ('inserted' as const),
        serverVersion: {
          minimum: null,
          newest: null,
          message: '',
          compatible: true,
        },
        ...host,
      };
    },
    setServerLabel: async (profile, label) => {
      const server = servers.find((entry) => entry.profileName === profile);
      if (!server)
        throw failure('profile-not-found', 'That server is not configured.');
      if (
        label?.includes('\0') ||
        label?.includes('\r') ||
        label?.includes('\n')
      )
        throw failure('invalid-request', 'Enter a valid display name.');
      const trimmed = label?.trim() ?? '';
      if (new TextEncoder().encode(trimmed).length > 64)
        throw failure('invalid-request', 'Enter a valid display name.');
      const normalized =
        trimmed === '' || trimmed === server.profileName ? null : trimmed;
      const changed = server.displayLabel !== normalized;
      server.displayLabel = normalized;
      return { profile, label: normalized, changed };
    },
    removeServerAndCredentials: async (profileName, confirmedProfileName) => {
      if (profileName !== confirmedProfileName)
        throw failure(
          'invalid-request',
          'Type the exact server name to remove it.',
        );
      const index = servers.findIndex(
        (server) => server.profileName === profileName,
      );
      if (index >= 0) servers.splice(index, 1);
      serverHosts.delete(profileName);
      serverProbes.delete(profileName);
      return { profile: profileName, removed: true as const };
    },
  };
}
