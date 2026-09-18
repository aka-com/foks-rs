import assert from 'node:assert/strict';
import test from 'node:test';
import {
  mergeProfileSnapshot,
  projectCatalogFreshness,
} from '../src/catalog-state';
import { FIXTURE } from '../src/fixture';
import { mockBridge } from '../src/mock-bridge';

test('concurrent profile publication merges only its own scope into the newest base', () => {
  const [a, b] = FIXTURE.servers;
  assert.ok(a && b);
  const earlier = {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server) =>
      server.id === a.id ? { ...server, label: 'A refreshed' } : server,
    ),
  };
  const later = {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server) =>
      server.id === b.id ? { ...server, label: 'B refreshed' } : server,
    ),
  };
  const result = mergeProfileSnapshot(later, earlier, a.id);
  assert.equal(result.servers.length, FIXTURE.servers.length);
  assert.equal(
    result.servers.find((server) => server.id === a.id)?.label,
    'A refreshed',
  );
  assert.equal(
    result.servers.find((server) => server.id === b.id)?.label,
    'B refreshed',
  );
  assert.equal(result.stores.length, FIXTURE.stores.length);
  assert.equal(result.items.length, FIXTURE.items.length);
});

test('retained store content does not count as a newly completed read', async () => {
  const response = await mockBridge(FIXTURE).listCatalog();
  const store = FIXTURE.stores[0];
  const base = {
    ...FIXTURE,
    catalogFreshness: {
      profiles: {},
      stores: { [store.id]: { refreshing: false, lastSuccessAt: 10 } },
    },
  };
  const freshness = projectCatalogFreshness(
    base,
    base,
    {
      ...response,
      fullItemReads: [],
      storeReads: [{ store: store.id, state: 'not-loaded' }],
    },
    true,
    20,
  );
  assert.equal(freshness.stores[store.id].lastSuccessAt, 10);
  assert.equal(freshness.stores[store.id].refreshing, true);
  const complete = projectCatalogFreshness(
    base,
    base,
    {
      ...response,
      fullItemReads: [],
      storeReads: [{ store: store.id, state: 'complete' }],
    },
    true,
    30,
  );
  assert.equal(complete.stores[store.id].lastSuccessAt, 30);
});
