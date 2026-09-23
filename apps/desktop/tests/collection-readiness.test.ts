import assert from 'node:assert/strict';
import test from 'node:test';
import { FIXTURE } from '../src/fixture';
import { mockBridge } from '../src/mock-bridge';
import { projectCatalog } from '../src/bridge/snapshot-projection';
import { mergeProfileSnapshot, failCatalogRefresh } from '../src/catalog-state';
import {
  groupDetailReadiness,
  inventoryReadiness,
  itemsReadiness,
  type AgentSnapshot,
} from '../src/model';

test('inventory distinguishes startup, completed empty reads and failed refreshes', () => {
  const pending: AgentSnapshot = {
    ...FIXTURE,
    stores: [],
    accounts: [],
    items: [],
    profileInventory: FIXTURE.profileInventory.map((row) => ({
      ...row,
      accounts: 'unavailable',
      teams: 'unavailable',
    })),
  };
  assert.equal(inventoryReadiness(pending, 'accounts'), 'loading');
  assert.equal(itemsReadiness(pending), 'loading');
  const failed = failCatalogRefresh(
    pending,
    pending.catalogProfiles,
    {
      code: 'io',
      message: 'Offline',
      retryable: true,
      ambiguous: false,
      fatal: false,
    },
    2,
  );
  assert.equal(inventoryReadiness(failed, 'teams'), 'unavailable');
  assert.equal(itemsReadiness(failed), 'unavailable');
  assert.equal(
    itemsReadiness({ ...pending, profileInventory: FIXTURE.profileInventory }),
    'ready',
  );
});

test('partial projections mark unread rosters and profile merges keep other teams readiness', async () => {
  const bridge = { ...mockBridge(FIXTURE), native: true };
  const catalog = await bridge.listCatalog();
  const partial = await projectCatalog(
    bridge,
    catalog,
    undefined,
    1,
    FIXTURE.agent,
    true,
  );
  const team = partial.stores.find(
    (store) => store.kind === 'team' && store.active,
  )!;
  assert.ok(team);
  assert.equal(groupDetailReadiness(partial, team.id, 'roster'), 'loading');
  assert.equal(groupDetailReadiness(partial, team.id, 'federation'), 'loading');
  const final = await projectCatalog(
    bridge,
    catalog,
    partial,
    2,
    FIXTURE.agent,
    false,
  );
  assert.equal(groupDetailReadiness(final, team.id, 'roster'), 'ready');
  assert.equal(groupDetailReadiness(final, team.id, 'federation'), 'ready');
  const merged = mergeProfileSnapshot(partial, final, team.server);
  for (const entry of partial.groupDetailInventory!) {
    const inScope =
      partial.stores.find((store) => store.id === entry.store)?.server ===
      team.server;
    assert.deepEqual(
      merged.groupDetailInventory!.find((row) => row.store === entry.store),
      (inScope ? final : partial).groupDetailInventory!.find(
        (row) => row.store === entry.store,
      ),
    );
  }
  const refreshing = await projectCatalog(
    bridge,
    catalog,
    final,
    3,
    FIXTURE.agent,
    true,
  );
  assert.equal(groupDetailReadiness(refreshing, team.id, 'roster'), 'ready');
});

test('completed item reads do not finish roster enrichment, but a failed attempt stops its loading state', () => {
  const team = FIXTURE.stores.find((store) => store.kind === 'team')!;
  const partial: AgentSnapshot = {
    ...FIXTURE,
    groupDetailInventory: [
      { store: team.id, roster: 'loading', federation: 'loading' },
    ],
    catalogFreshness: {
      profiles: { [team.server]: { refreshing: false, lastSuccessAt: 1 } },
      stores: {},
    },
  };
  assert.equal(groupDetailReadiness(partial, team.id, 'roster'), 'loading');
  const failed = failCatalogRefresh(
    partial,
    partial.catalogProfiles,
    {
      code: 'io',
      message: 'Offline',
      retryable: true,
      ambiguous: false,
      fatal: false,
    },
    2,
  );
  assert.equal(groupDetailReadiness(failed, team.id, 'roster'), 'unavailable');
});

test('a known vault can finish independently of the remaining team inventory', () => {
  const account = FIXTURE.stores.find((store) => store.kind === 'account')!;
  const snapshot: AgentSnapshot = {
    ...FIXTURE,
    profileInventory: FIXTURE.profileInventory.map((entry) => ({
      ...entry,
      teams: 'unavailable',
    })),
  };
  assert.equal(itemsReadiness(snapshot), 'loading');
  assert.equal(itemsReadiness(snapshot, account.id), 'ready');
});
