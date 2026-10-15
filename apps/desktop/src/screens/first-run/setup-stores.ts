import type { FirstRunCheckpoint } from '../../first-run-state';
import { storeReadable } from '../../model';
import type { AgentSnapshot } from '../../model';

/**
 * What the end steps show from the catalog: the account's Personal vault and
 * the team vault setup joined, each matched exactly to the checkpoint's server,
 * account and team identity. More than one match is no match.
 */
export function setupStores(
  snapshot: AgentSnapshot,
  checkpoint: FirstRunCheckpoint,
  facts?: { admin?: string; groupName?: string },
) {
  const profile = checkpoint.profile;
  const addedStores = checkpoint.group
    ? snapshot.stores.filter(
        (store) =>
          store.kind === 'team' &&
          store.active &&
          storeReadable(snapshot, store.id) &&
          store.server === profile?.profile &&
          store.account === checkpoint.account?.alias &&
          store.alias === checkpoint.group?.alias &&
          store.team_kind === checkpoint.group?.kind &&
          store.team_id_hex === checkpoint.group?.teamIdHex,
      )
    : [];
  const accountStores =
    checkpoint.account && profile
      ? snapshot.accounts.filter(
          (candidate) =>
            candidate.server === profile.profile &&
            candidate.alias === checkpoint.account?.alias,
        )
      : [];
  const accountStore =
    accountStores.length === 1 ? accountStores[0]?.store : undefined;
  return {
    admin: facts?.admin ?? 'team administrator',
    adminShort: facts?.admin
      ? facts.admin.split('.')[0]
      : 'the team administrator',
    group: checkpoint.group?.name ?? facts?.groupName ?? 'your team',
    addedStore: addedStores.length === 1 ? addedStores[0]?.id : undefined,
    accountStore,
    personalAvailable: Boolean(
      accountStore && storeReadable(snapshot, accountStore),
    ),
    accountStoreRecord: accountStore
      ? snapshot.stores.find((candidate) => candidate.id === accountStore)
      : undefined,
    accountItemCount: accountStore
      ? snapshot.items.filter((item) => item.store === accountStore).length
      : 0,
  };
}

export type SetupStores = ReturnType<typeof setupStores>;
