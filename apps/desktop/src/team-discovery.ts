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

export async function reconcileTeamDiscovery(
  bridge: Bridge,
  account: Account,
  context: ReconciliationContext,
  eligible: () => boolean,
  reconcile: () => Promise<void>,
): Promise<void> {
  let dispatched = false;
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
  if (dispatched && context.isCurrent()) {
    try {
      await reconcile();
    } catch (error) {
      failure ??= error;
    }
  }
  if (failure !== undefined) throw failure;
}
