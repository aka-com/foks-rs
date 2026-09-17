import type { World } from './model';
import { transitionFirstRun, type FirstRunCheckpoint } from './first-run-state';

/** Adopt only a unique account from the acknowledged operation's pinned host. */
export function resolveProvisionedIdentity(
  world: World,
  checkpoint: FirstRunCheckpoint,
): FirstRunCheckpoint {
  const pending = checkpoint.provisionedAccount;
  const profile = checkpoint.profile;
  if (!pending || !profile) return checkpoint;
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
