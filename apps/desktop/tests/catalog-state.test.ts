import assert from 'node:assert/strict';
import test from 'node:test';
import {
  failCatalogRefresh,
  failWholeCatalogRefresh,
  markCatalogRefresh,
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

test('a whole-read failure preserves successful partial observations and prior scoped failures', async () => {
  const response = await mockBridge(FIXTURE).listCatalog();
  const [healthy, failed] = FIXTURE.servers;
  const scoped = {
    code: 'server-unavailable',
    message: 'One server unavailable.',
    retryable: true,
    fatal: false,
    ambiguous: false,
  };
  const root = {
    ...scoped,
    code: 'invalid-response',
    message: 'Catalog projection failed.',
  };
  const base = {
    ...FIXTURE,
    catalogFreshness: projectCatalogFreshness(
      FIXTURE,
      undefined,
      response,
      false,
      10,
    ),
  };
  const failedProfile = failCatalogRefresh(base, [failed.id], scoped, 20);
  const started = markCatalogRefresh(failedProfile, undefined, 30);
  assert.strictEqual(
    started.catalogFreshness?.profiles[failed.id].error,
    scoped,
  );
  const pending = projectCatalogFreshness(
    started,
    started,
    { ...response, fullItemReads: [] },
    true,
    30,
  );
  assert.strictEqual(pending.profiles[failed.id].error, scoped);
  const partial = {
    ...started,
    catalogFreshness: {
      ...started.catalogFreshness,
      profiles: {
        ...started.catalogFreshness.profiles,
        [healthy.id]: {
          refreshing: false,
          lastAttemptAt: 30,
          lastSuccessAt: 30,
        },
      },
    },
  };
  const terminal = failWholeCatalogRefresh(partial, root, 31);
  assert.strictEqual(terminal.catalogFreshness?.attempt?.error, root);
  assert.equal(terminal.catalogFreshness?.attempt?.lastSuccessAt, 10);
  assert.strictEqual(
    terminal.catalogFreshness?.profiles[healthy.id],
    partial.catalogFreshness.profiles[healthy.id],
  );
  assert.strictEqual(
    terminal.catalogFreshness?.profiles[failed.id].error,
    scoped,
  );
  assert.ok(
    Object.values(terminal.catalogFreshness.stores).every(
      (entry) => entry.error !== root && !entry.refreshing,
    ),
  );
  assert.strictEqual(terminal.notifications, base.notifications);
  assert.strictEqual(terminal.storeInventory, base.storeInventory);
  assert.strictEqual(terminal.items, base.items);
  const scopedSuccess = mergeProfileSnapshot(terminal, base, healthy.id);
  assert.strictEqual(
    scopedSuccess.catalogFreshness?.attempt,
    terminal.catalogFreshness?.attempt,
  );
  assert.strictEqual(
    markCatalogRefresh(terminal, [healthy.id], 32).catalogFreshness?.attempt,
    terminal.catalogFreshness?.attempt,
  );
});

test('only a completed whole read advances whole-catalog success', async () => {
  const response = await mockBridge(FIXTURE).listCatalog();
  const error = {
    code: 'io',
    message: 'Read failed.',
    retryable: false,
    fatal: false,
    ambiguous: false,
  };
  const base = failWholeCatalogRefresh(FIXTURE, error, 10);
  const partial = projectCatalogFreshness(base, base, response, true, 20);
  assert.equal(partial.attempt?.refreshing, true);
  assert.equal(partial.attempt?.lastSuccessAt, undefined);
  const scoped = projectCatalogFreshness(
    base,
    base,
    response,
    false,
    20,
    response.profiles[0],
  );
  assert.equal(scoped.attempt, undefined);
  const final = projectCatalogFreshness(base, base, response, false, 20);
  assert.equal(final.attempt?.refreshing, false);
  assert.equal(final.attempt?.lastSuccessAt, 20);
  assert.equal(final.attempt?.error, undefined);
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
