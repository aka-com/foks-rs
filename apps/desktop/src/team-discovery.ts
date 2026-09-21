import { scheduleProfileWork } from './scheduling/profile-work';
import type { ReconciliationContext } from './scheduling/reconciliation';
import type { Bridge } from './bridge';
import { storeOperationAvailability } from './model';
import type { Account, AgentSnapshot } from './model';

export function discoveryAccounts(
  snapshot: AgentSnapshot,
  nowSeconds = Date.now() / 1_000,
): readonly Account[] {
  return snapshot.accounts.filter((account) => {
    const store = snapshot.stores.find(
      (candidate) =>
        candidate.kind === 'account' &&
        candidate.id === account.store &&
        candidate.server === account.server &&
        candidate.account === account.alias,
    );
    return (
      store !== undefined &&
      snapshot.profileInventory.some(
        (entry) =>
          entry.profile === account.server && entry.accounts === 'complete',
      ) &&
      storeOperationAvailability(snapshot, store, 'teams', { nowSeconds })
        .available
    );
  });
}

/**
 * Whether the catalog already binds a team to this account.
 *
 * Binding is what discovery does, so an account the catalog holds no team
 * for has everything still to discover, while one whose teams are all bound
 * has only a team joined since the last run — which a sweep at a longer
 * interval finds. See `DISCOVERY_SWEEP_RUNS` in `desktop-reconciliation`.
 */
export function accountHasBoundTeam(
  snapshot: AgentSnapshot,
  account: Account,
): boolean {
  return snapshot.stores.some(
    (store) =>
      store.kind === 'team' &&
      store.server === account.server &&
      store.account === account.alias,
  );
}

export async function reconcileTeamDiscovery(
  bridge: Bridge,
  account: Account,
  context: ReconciliationContext,
  eligible: () => boolean,
  reconcile: () => Promise<void>,
): Promise<void> {
  let dispatched = false;
  // An older agent reports no bound list. That is unknown, not "nothing
  // changed", so the catalog read keeps happening as it always did.
  let wroteBinding = true;
  let failure: unknown;
  try {
    await scheduleProfileWork(
      bridge,
      account.server,
      async () => {
        if (!context.isCurrent() || !eligible())
          throw Object.assign(
            new Error('Team discovery was retired before dispatch.'),
            {
              code: 'cancelled',
              retryable: false,
              ambiguous: false,
              fatal: false,
            },
          );
        dispatched = true;
        const result = await bridge.discoverGroups(
          account.server,
          account.alias,
        );
        if (result.accountAlias !== account.alias) {
          throw Object.assign(
            new Error('Team discovery returned a different account.'),
            {
              code: 'response-binding',
              fatal: true,
              ambiguous: true,
              retryable: false,
            },
          );
        }
        if (result.bound !== undefined) wroteBinding = result.bound.length > 0;
      },
      {
        key: `team-discovery:${account.store}`,
        owner: context,
        generation: 0,
        signal: context.signal,
        current: context.isCurrent,
        cancel: () => undefined,
        preemptible: false,
      },
    );
  } catch (error) {
    failure = error;
  }
  // The periodic job runs for every account, because it is how being added
  // to a team is noticed. The catalog walk that follows it is only worth its
  // cost when discovery actually wrote a binding.
  if (dispatched && wroteBinding && context.isCurrent()) {
    try {
      await reconcile();
    } catch (error) {
      failure ??= error;
    }
  }
  if (failure !== undefined) throw failure;
}
