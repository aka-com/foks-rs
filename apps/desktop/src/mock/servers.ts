/**
 * The mock's server commands, the members of `bridge/commands-servers.ts`,
 * over the shared server records. All but the server listing answer at the
 * wire level through the mock host.
 */

import type { Bridge } from '../bridge';
import type { serverCommands } from '../bridge/commands-servers';
import { mockCommand, type MockCommand } from './host';
import { failure, HOST_IDS, type MockDomain, type MockState } from './state';
import { serverStatusWire } from './wire';

/**
 * The server members whose commands the mock host answers at the wire level,
 * through `bridge/commands-servers.ts`.
 */
export const SERVER_HOST_MEMBERS = [
  'checkAndAddProfile',
  'checkAndAddGoProfile',
  'describeServerStatus',
  'checkServer',
  'setServerLabel',
  'removeServerAndCredentials',
] as const satisfies readonly (keyof ReturnType<typeof serverCommands>)[];

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

/**
 * The commands of `SERVER_HOST_MEMBERS`: adding, checking, labelling and
 * removing a server profile, and its status, answered with wire JSON.
 */
export function mockServerHost(state: MockState): Record<string, MockCommand> {
  const { servers, serverProbes, serverHosts, firstRunFixture } = state;
  const describeServerStatus = mockServerStatus(state);
  return {
    check_and_add_profile: mockCommand(
      async ({
        profileName,
        probe,
      }: {
        profileName: string;
        probe: string;
      }) => {
        const path =
          probe === firstRunFixture.invited.server
            ? firstRunFixture.invited
            : probe === firstRunFixture.own.server
              ? firstRunFixture.own
              : undefined;
        if (!path) throw failure('io', `No response from ${probe}`);
        return { ...path.report, profile: profileName };
      },
    ),
    check_and_add_go_profile: mockCommand(
      async ({
        hostId,
        profileName,
        probe,
      }: {
        hostId: string;
        profileName: string;
        probe: string;
      }) => {
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
    ),
    describe_server_status: mockCommand(
      async ({ profile }: { profile: string }) =>
        serverStatusWire(await describeServerStatus(profile)),
    ),
    check_server: mockCommand(async ({ profile }: { profile: string }) => {
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
    }),
    set_server_label: mockCommand(
      async ({ profile, label }: { profile: string; label: string | null }) => {
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
    ),
    remove_server_and_credentials: mockCommand(
      async ({
        profileName,
        confirmedProfileName,
      }: {
        profileName: string;
        confirmedProfileName: string;
      }) => {
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
    ),
  };
}

export function mockServerCommands(
  state: MockState,
): MockDomain<typeof serverCommands, (typeof SERVER_HOST_MEMBERS)[number]> {
  const { servers } = state;
  return {
    // An optional method only the native host offers; callers fall back to
    // other paths when a bridge lacks it.
    reconcileServer: undefined,
    // The fixture's servers carry trust, lease and service facts that the
    // native list_servers response does not, so this listing stays decoded.
    listServers: () =>
      Promise.resolve(servers.map((server) => ({ ...server }))),
  };
}
