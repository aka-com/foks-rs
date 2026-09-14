import type { World } from './model';
import { transitionFirstRun, type FirstRunCheckpoint } from './first-run-state';

export type IdentityProblem =
  | 'inventory-unavailable'
  | 'profile-missing'
  | 'host-mismatch'
  | 'account-missing'
  | 'duplicate-records';

export const identityProblemText: Record<IdentityProblem, string> = {
  'inventory-unavailable':
    'Couldn’t load your account details. Try again, or finish setup later.',
  'profile-missing': 'The server saved for this account is missing.',
  'host-mismatch':
    'The saved server identity no longer matches this server. Review your server settings to continue.',
  'account-missing':
    'FOKS refreshed its account list but couldn’t find the connected account. Review your account settings or check again.',
  'duplicate-records':
    'The local account records conflict. Review your account and server settings to continue.',
};

export function provisionedIdentityProblem(
  world: World,
  checkpoint: FirstRunCheckpoint,
): IdentityProblem | null {
  const profile = checkpoint.profile;
  const pending = checkpoint.provisionedAccount;
  if (!profile || !pending) return null;
  if (world.profileInventoryStatus !== 'complete')
    return 'inventory-unavailable';
  const servers = world.servers.filter(
    (server) => server.id === profile.profile,
  );
  if (servers.length === 0) return 'profile-missing';
  if (servers.length !== 1) return 'duplicate-records';
  if (!servers[0].host_id) return 'inventory-unavailable';
  if (servers[0].host_id !== profile.hostId) return 'host-mismatch';
  if (
    world.profileInventory.find((row) => row.profile === profile.profile)
      ?.accounts !== 'complete'
  )
    return 'inventory-unavailable';
  const matches = world.accounts.filter(
    (account) =>
      account.server === profile.profile && account.alias === pending.alias,
  );
  if (!matches.length) return 'account-missing';
  if (matches.length !== 1) return 'duplicate-records';
  return null;
}

/** Adopt only a unique account from the acknowledged operation's pinned host. */
export function resolveProvisionedIdentity(
  world: World,
  checkpoint: FirstRunCheckpoint,
): FirstRunCheckpoint {
  const pending = checkpoint.provisionedAccount;
  const profile = checkpoint.profile;
  if (!pending || !profile) return checkpoint;
  if (provisionedIdentityProblem(world, checkpoint)) return checkpoint;
  const servers = world.servers.filter(
    (server) => server.id === profile.profile,
  );
  if (servers.length !== 1 || servers[0]?.host_id !== profile.hostId)
    return checkpoint;
  if (
    world.profileInventory.find((row) => row.profile === profile.profile)
      ?.accounts !== 'complete'
  )
    return checkpoint;
  const matches = world.accounts.filter(
    (account) =>
      account.server === profile.profile && account.alias === pending.alias,
  );
  if (matches.length !== 1) return checkpoint;
  return transitionFirstRun(checkpoint, {
    type: 'account-complete',
    alias: pending.alias,
    username: matches[0].username,
    deviceName: pending.deviceName,
  });
}
