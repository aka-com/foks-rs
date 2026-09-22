import type { AgentSnapshot } from './model';
import { profileInventoryComplete } from './model';
import { resolveProvisionedIdentity } from './first-run-identity';
import {
  initialFirstRun,
  normalizeFirstRunCheckpoint,
  reconcileFirstRunCheckpoint,
  type FirstRunCheckpoint,
  type FirstRunStateName,
} from './first-run-state';

export function authoritativeSetupFacts(
  snapshot: AgentSnapshot,
  checkpoint: FirstRunCheckpoint,
): Parameters<typeof reconcileFirstRunCheckpoint>[1] {
  const inventory = snapshot.profileInventory.find(
    (row) => row.profile === checkpoint.profile?.profile,
  );
  const server = checkpoint.profile
    ? snapshot.servers.find(
        (candidate) => candidate.profileName === checkpoint.profile?.profile,
      )
    : undefined;
  const profile = checkpoint.profile
    ? !profileInventoryComplete(snapshot, 'profiles')
      ? 'unknown'
      : !server
        ? 'missing'
        : server.host_id === null
          ? 'unknown'
          : server.host_id === checkpoint.profile.hostId
            ? 'present'
            : 'missing'
    : 'unknown';
  const account = checkpoint.account
    ? snapshot.profileInventoryStatus === 'complete' &&
      inventory?.accounts === 'complete'
      ? snapshot.accounts.some(
          (candidate) =>
            candidate.server === checkpoint.profile?.profile &&
            candidate.alias === checkpoint.account?.alias,
        )
        ? 'present'
        : 'missing'
      : 'unknown'
    : 'unknown';
  const group = checkpoint.group
    ? snapshot.profileInventoryStatus === 'complete' &&
      inventory?.teams === 'complete'
      ? snapshot.stores.some(
          (candidate) =>
            candidate.kind === 'team' &&
            candidate.account === checkpoint.account?.alias &&
            candidate.server === checkpoint.profile?.profile &&
            candidate.alias === checkpoint.group?.alias &&
            candidate.team_id_hex === checkpoint.group.teamIdHex,
        )
        ? 'present'
        : 'missing'
      : 'unknown'
    : 'unknown';
  return { profile, account, group };
}

export function reconcileSetup(
  snapshot: AgentSnapshot,
  checkpoint: FirstRunCheckpoint,
): FirstRunCheckpoint {
  return normalizeFirstRunCheckpoint(
    resolveProvisionedIdentity(
      snapshot,
      reconcileFirstRunCheckpoint(
        checkpoint,
        authoritativeSetupFacts(snapshot, checkpoint),
      ),
    ),
  );
}

/** All production entry points pass through the same prerequisite checks. */
export function resolveSetupEntry(
  snapshot: AgentSnapshot,
  saved: FirstRunCheckpoint | null,
  requested: { state: FirstRunStateName; path?: FirstRunCheckpoint['path'] },
  automatic = false,
): FirstRunCheckpoint {
  if (saved?.provisioning || saved?.provisionedAccount || (automatic && saved))
    return reconcileSetup(snapshot, saved);
  requested = {
    ...requested,
    state: requested.state === 'phrase' ? 'protect' : requested.state,
  };
  const path = requested.path ?? saved?.path ?? 'invited';
  if (
    !saved ||
    (requested.path && requested.path !== saved.path) ||
    (requested.state === 'who' && saved.state !== 'who')
  )
    return reconcileSetup(snapshot, initialFirstRun(path, requested.state));
  return reconcileSetup(snapshot, { ...saved, path, state: requested.state });
}

/** Shared policy for page and sidebar recovery actions. Reads may be abandoned. */
export function setupActions(
  checkpoint: FirstRunCheckpoint,
  activity: {
    mutating: boolean;
    operationRunning: boolean;
    inFlight: boolean;
  },
) {
  const restartBlocked =
    activity.mutating || activity.operationRunning || activity.inFlight;
  return {
    canRestart: !restartBlocked,
    restartReason: restartBlocked
      ? 'Wait for the current operation to finish. You can finish setup later.'
      : 'Keeps your existing accounts and server settings.',
    canChooseAnotherAccount:
      !restartBlocked &&
      Boolean(checkpoint.provisioning || checkpoint.provisionedAccount),
  };
}
