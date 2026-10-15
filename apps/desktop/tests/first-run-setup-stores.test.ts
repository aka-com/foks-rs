import assert from 'node:assert/strict';
import test from 'node:test';
import { FIXTURE } from '../src/fixture';
import type { TeamStore } from '../src/model';
import {
  initialFirstRun,
  type FirstRunCheckpoint,
} from '../src/first-run-state';
import { setupStores } from '../src/screens/first-run/setup-stores';

const household = FIXTURE.stores.find(
  (store): store is TeamStore =>
    store.kind === 'team' && store.id === 'team:household',
);
if (!household) throw new Error('The fixture lacks the household team.');
const profile = {
  profile: 'personal',
  hostId: `02${'2'.repeat(64)}`,
  lookupName: 'localhost',
  canonicalName: 'localhost',
  acceptance: 'unchanged' as const,
  chain: 1,
  epoch: 1,
};
const joined: FirstRunCheckpoint = {
  ...initialFirstRun('invited', 'added'),
  profile,
  account: { alias: 'personal', username: 'satoshi', deviceName: 'Mac' },
  group: {
    name: 'Household',
    kind: 'named',
    alias: 'household',
    teamIdHex: household.team_id_hex,
  },
  added: true,
};

test('the end steps find the joined team and the Personal vault by exact identity', () => {
  const stores = setupStores(FIXTURE, joined);
  assert.equal(stores.addedStore, 'team:household');
  assert.equal(stores.accountStore, 'acct:personal');
  assert.equal(stores.personalAvailable, true);
  assert.equal(stores.accountStoreRecord?.id, 'acct:personal');
  assert.equal(
    stores.accountItemCount,
    FIXTURE.items.filter((item) => item.store === 'acct:personal').length,
  );
  assert.equal(stores.group, 'Household');
});

test('a team that differs in any identity field is not the joined team', () => {
  for (const group of [
    { ...joined.group!, alias: 'homelab' },
    { ...joined.group!, kind: 'adhoc' as const },
    { ...joined.group!, teamIdHex: `03${'0'.repeat(64)}` },
  ])
    assert.equal(
      setupStores(FIXTURE, { ...joined, group }).addedStore,
      undefined,
    );
  assert.equal(
    setupStores(FIXTURE, {
      ...joined,
      account: { ...joined.account!, alias: 'work' },
    }).addedStore,
    undefined,
  );
});

test('two matching vaults are no match', () => {
  // A second readable copy of the joined team, and a second account row.
  const copy = 'team:household-copy';
  const twice = {
    ...FIXTURE,
    stores: [...FIXTURE.stores, { ...household, id: copy }],
    storeInventory: [
      ...FIXTURE.storeInventory,
      ...FIXTURE.storeInventory
        .filter((row) => row.store === household.id)
        .map((row) => ({ ...row, store: copy })),
    ],
    parties: [
      ...FIXTURE.parties,
      ...FIXTURE.parties
        .filter((row) => row.store === household.id)
        .map((row) => ({ ...row, store: copy })),
    ],
    accounts: [...FIXTURE.accounts, { ...FIXTURE.accounts[0] }],
  };
  const stores = setupStores(twice, joined);
  assert.equal(stores.addedStore, undefined);
  assert.equal(stores.accountStore, undefined);
  assert.equal(stores.personalAvailable, false);
  assert.equal(stores.accountItemCount, 0);
});

test('team and administrator names fall back to the review facts, then to generic text', () => {
  const unnamed = { ...joined, group: undefined };
  assert.deepEqual(
    (({ admin, adminShort, group }) => ({ admin, adminShort, group }))(
      setupStores(FIXTURE, unnamed),
    ),
    {
      admin: 'team administrator',
      adminShort: 'the team administrator',
      group: 'your team',
    },
  );
  const named = setupStores(FIXTURE, unnamed, {
    admin: 'ana.lopez',
    groupName: 'Platform',
  });
  assert.equal(named.admin, 'ana.lopez');
  assert.equal(named.adminShort, 'ana');
  assert.equal(named.group, 'Platform');
});
