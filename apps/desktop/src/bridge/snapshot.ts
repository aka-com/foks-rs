import {
  refreshActivitiesFor,
  type RefreshOperation,
} from '../refresh-activity';
import { mergeProfileSnapshot } from '../catalog-state';
import { CatalogReadRetiredError } from '../catalog-coordinator';
import { profileInventoryComplete, serverFactAvailability } from '../model';
import type { AgentSnapshot, AgentStatus } from '../model';
import {
  scheduleProfileWork,
  type BackgroundHistoryWork,
} from '../scheduling/profile-work';
import type { Bridge } from './contract';
import type { CatalogDto } from './vault-catalog';
import {
  normalizeCommandError,
  isTerminalCommandError,
  reportReadinessError,
  type CommandError,
} from './errors';
import { enqueueProfileWork } from './profile-work';
import { projectCatalog } from './snapshot-projection';

/**
 * Runs authenticated team discovery for each account store that is not bound to
 * any team yet, so a user who is added to a group after setup sees it on the
 * next ordinary launch. Discovery is best-effort: an unavailable server or a
 * profile that cannot be opened remains unbound until a later discovery attempt.
 *
 * Returns true when any discovery was attempted. The native command invalidates
 * the catalog even if no groups are found or the operation fails, so the
 * catalog must be reloaded after all of those cases.
 */
export async function discoverUnboundTeams(
  bridge: Bridge,
  snapshot: AgentSnapshot,
  isCurrent: () => boolean = () => true,
): Promise<boolean> {
  if (!profileInventoryComplete(snapshot, 'accounts')) return false;
  const bound = new Set<string>();
  for (const store of snapshot.stores) {
    if (store.kind === 'team')
      bound.add(`${store.server}\u0000${store.account}`);
  }
  let attempted = false;
  for (const account of snapshot.accounts) {
    if (!isCurrent()) break;
    if (bound.has(`${account.server}\u0000${account.alias}`)) continue;
    if (
      !snapshot.stores.some(
        (store) =>
          store.kind === 'account' &&
          store.id === account.store &&
          store.server === account.server &&
          store.account === account.alias,
      )
    )
      continue;
    const server = snapshot.servers.find(
      (entry) => entry.profileName === account.server,
    );
    if (
      !server ||
      !serverFactAvailability(
        server,
        snapshot.observedExpiredLeases,
        { agentReady: snapshot.agent.state === 'ready' },
        ['teams'],
      ).available
    )
      continue;
    attempted = true;
    try {
      await enqueueProfileWork(
        bridge,
        account.server,
        async () => {
          if (isCurrent())
            await bridge.discoverGroups(account.server, account.alias);
        },
        'discover-groups',
      );
    } catch {
      // Best effort: leave the account unbound until a later launch.
    }
  }
  return attempted;
}

/** Load the current snapshot state. */
/**
 * Loads the snapshot from the native catalog. The load is a sequence of reads
 * bound to one catalog snapshot; when a concurrent load or mutation replaces
 * that snapshot mid-sequence the native side reports catalog-required, and
 * one further attempt is made against the new snapshot before giving up.
 */
export async function loadSnapshot(
  bridge: Bridge,
  base = bridge.fixtureSnapshot,
  nowSeconds: number = Math.floor(Date.now() / 1000),
  onPartial?: (snapshot: AgentSnapshot) => void,
  isCurrent: () => boolean = () => true,
  /** Read every roster, for a refresh the user asked for. */
  forceRosters = false,
  /**
   * The readiness a caller already established, so a boot does not probe the
   * agent twice. Absent means probe, as every other caller does.
   */
  ready?: AgentStatus,
  activity?: RefreshOperation,
): Promise<AgentSnapshot> {
  const operation =
    activity ??
    refreshActivitiesFor(bridge).begin('Loading catalog', isCurrent);
  try {
    return await loadSnapshotTracked(
      bridge,
      base,
      nowSeconds,
      onPartial,
      isCurrent,
      forceRosters,
      ready,
      operation,
    );
  } finally {
    if (!activity) operation.finish();
  }
}

async function loadSnapshotTracked(
  bridge: Bridge,
  base: AgentSnapshot | undefined,
  nowSeconds: number,
  onPartial: ((snapshot: AgentSnapshot) => void) | undefined,
  isCurrent: () => boolean,
  forceRosters: boolean,
  ready: AgentStatus | undefined,
  activity: RefreshOperation,
): Promise<AgentSnapshot> {
  try {
    return await loadSnapshotOnce(
      bridge,
      base,
      nowSeconds,
      onPartial,
      isCurrent,
      forceRosters,
      ready,
      activity,
    );
  } catch (error) {
    if (
      !isCurrent() ||
      normalizeCommandError(error).code !== 'catalog-required'
    )
      throw error;
    return loadSnapshotOnce(
      bridge,
      base,
      nowSeconds,
      onPartial,
      isCurrent,
      forceRosters,
      ready,
      activity,
    );
  }
}

async function loadSnapshotOnce(
  bridge: Bridge,
  base: AgentSnapshot | undefined,
  nowSeconds: number,
  onPartial: ((snapshot: AgentSnapshot) => void) | undefined,
  isCurrent: () => boolean,
  forceRosters: boolean,
  ready: AgentStatus | undefined,
  activity: RefreshOperation,
): Promise<AgentSnapshot> {
  const startedAt = performance.now();
  if (!isCurrent()) throw new CatalogReadRetiredError();
  activity.update('Checking local agent');
  const agent = ready ?? (await bridge.agentStatus());
  if (!isCurrent()) throw new CatalogReadRetiredError();
  if (agent.state !== 'ready') {
    const error: CommandError = {
      code: 'bootstrap-required',
      message:
        'The local agent must finish initialization before loading vaults.',
      retryable: false,
      ambiguous: false,
      fatal: false,
      details: { reason: agent.step },
    };
    reportReadinessError(error);
    throw error;
  }
  let accepting = true;
  let revision = 0;
  let terminalFailure: CommandError | undefined;
  /**
   * The newest partial catalog that has not been projected yet, and the chain
   * that drains it.
   *
   * Each partial carries the whole accumulated catalog, so projecting an
   * older one is work whose answer the next projection replaces in full. Only
   * the newest is kept: a partial that lands while a projection runs replaces
   * whatever was waiting, and the drain picks it up when that projection ends.
   */
  let queued: CatalogDto | undefined;
  let draining: Promise<void> | undefined;
  /**
   * Projects queued partials until none is left. The revision is taken here,
   * when a projection starts, rather than when its partial arrived: taken on
   * arrival it would always name the newest partial, and a projection the
   * next partial superseded would still publish.
   */
  const drain = async (): Promise<void> => {
    while (queued) {
      const partial = queued;
      queued = undefined;
      if (!accepting || !isCurrent() || terminalFailure) return;
      const current = ++revision;
      try {
        const snapshot = await projectCatalog(
          bridge,
          partial,
          base,
          nowSeconds,
          agent,
          true,
          undefined,
          undefined,
          false,
          activity,
        );
        if (
          accepting &&
          isCurrent() &&
          current === revision &&
          !terminalFailure
        )
          onPartial?.(snapshot);
      } catch (cause: unknown) {
        if (!isCurrent()) return;
        const error = normalizeCommandError(cause);
        if (isTerminalCommandError(error)) terminalFailure ??= error;
      }
    }
  };
  try {
    activity.update('Loading catalog');
    const response = await bridge.listCatalog(
      onPartial
        ? (partial) => {
            if (!accepting || !isCurrent() || terminalFailure) return;
            queued = partial;
            draining = (draining ?? Promise.resolve()).then(drain);
          }
        : undefined,
      // A refresh the user asked for, and a read back of a write, must not be
      // answered from the first page the agent retained before it.
      forceRosters,
    );
    accepting = false;
    // A partial the drain never took cannot publish now that `accepting` is
    // false, so projecting it would only delay the final read.
    queued = undefined;
    activity.update('Loading catalog details');
    await draining;
    if (!isCurrent()) throw new CatalogReadRetiredError();
    if (terminalFailure) throw terminalFailure;
    const snapshot = await projectCatalog(
      bridge,
      response,
      base,
      nowSeconds,
      agent,
      false,
      undefined,
      undefined,
      forceRosters,
      activity,
      startedAt,
    );
    if (!isCurrent()) throw new CatalogReadRetiredError();
    return snapshot;
  } finally {
    accepting = false;
  }
}

export async function loadProfileSnapshot(
  bridge: Bridge,
  profile: string,
  base: AgentSnapshot,
  nowSeconds: number = Math.floor(Date.now() / 1000),
  isCurrent: () => boolean = () => true,
  background?: BackgroundHistoryWork,
  /** Read this profile's rosters whether or not a team chain moved. */
  forceRosters = false,
): Promise<AgentSnapshot> {
  const startedAt = performance.now();
  if (!isCurrent()) throw new CatalogReadRetiredError();
  const response = await scheduleProfileWork(
    bridge,
    profile,
    () => bridge.listProfileCatalog(profile),
    background,
    'profile-catalog',
  );
  if (!isCurrent()) throw new CatalogReadRetiredError();
  if (
    response.profiles.length !== 1 ||
    response.profiles[0] !== profile ||
    [...response.stores, ...response.knownStores].some(
      (store) => store.server !== profile,
    ) ||
    response.inventory.some((entry) => entry.profile !== profile) ||
    response.failures.some((entry) => entry.profile !== profile) ||
    response.blockedProfiles.some((entry) => entry !== profile) ||
    response.fullItemReads?.some((entry) => entry !== profile) ||
    response.localMetadata?.profiles.some(
      (entry) => entry.profile !== profile,
    ) ||
    response.localMetadata?.accounts.some((entry) => entry.server !== profile)
  )
    throw new Error('list_profile_catalog returned a different scope.');
  if (
    bridge.native &&
    (!response.localMetadata || response.localMetadata.profiles.length !== 1)
  )
    throw new Error('list_profile_catalog omitted scoped metadata.');
  const projected = await projectCatalog(
    bridge,
    response,
    base,
    nowSeconds,
    base.agent,
    false,
    profile,
    background,
    forceRosters,
    undefined,
    startedAt,
  );
  if (!isCurrent()) throw new CatalogReadRetiredError();
  return mergeProfileSnapshot(base, projected, profile);
}
