import assert from 'node:assert/strict';
import test from 'node:test';
import { loadSnapshot, type Bridge, type CatalogDto } from '../src/bridge';
import { FIXTURE } from '../src/fixture';
import { mockBridge } from '../src/mock-bridge';
import {
  profileInventoryComplete,
  serverCapabilityAvailability,
  storeAvailability,
  type AgentSnapshot,
} from '../src/model';
import { failCatalogRefresh, markCatalogRefresh } from '../src/catalog-state';

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

test('partial projection publishes healthy items without any unfinished metadata enrichment', async () => {
  const base = mockBridge(FIXTURE);
  const catalog = await base.listCatalog();
  const profile = catalog.stores.find(
    (store) => store.kind === 'account',
  )!.server;
  const stores = catalog.stores.filter((store) => store.server === profile);
  const ids = new Set(stores.map((store) => store.id));
  const partial: CatalogDto = {
    ...catalog,
    profiles: [profile, 'unfinished'],
    fullItemReads: [profile],
    stores,
    knownStores: stores,
    inventory: catalog.inventory.filter((entry) => entry.profile === profile),
    items: catalog.items.filter((item) => ids.has(item.store)),
    failures: [],
    blockedProfiles: [],
    localMetadata: {
      accounts: (await base.listAccounts()).filter((account) =>
        ids.has(account.store),
      ),
      profiles: [
        {
          profile,
          label: null,
          configuredProbe: profile,
          status: await base.describeServerStatus(profile),
          error: null,
        },
      ],
    },
  };
  const gate = deferred<CatalogDto>();
  const published = deferred<AgentSnapshot>();
  let emit!: (catalog: CatalogDto) => void;
  const bridge: Bridge = {
    ...base,
    native: true,
    fixtureSnapshot: undefined,
    listCatalog: (onPartial) => {
      emit = onPartial!;
      return gate.promise;
    },
    listServers: async () => {
      throw new Error('must not enrich unfinished catalog');
    },
    listAccounts: async () => {
      throw new Error('must not enrich unfinished catalog');
    },
    describeServerStatus: async () => {
      throw new Error('must not enrich unfinished catalog');
    },
    listGroupDetails: async () => {
      throw new Error('must not enrich unfinished catalog');
    },
  };
  let current = true;
  const pending = loadSnapshot(
    bridge,
    undefined,
    1,
    published.resolve,
    () => current,
  );
  await Promise.resolve();
  emit(partial);
  const snapshot = await Promise.race([
    published.promise,
    new Promise<never>((_, reject) =>
      setTimeout(() => reject(new Error('no partial publication')), 500),
    ),
  ]);
  assert.ok(snapshot.items.length > 0);
  assert.equal(profileInventoryComplete(snapshot, 'accounts'), false);
  assert.equal(
    snapshot.profileInventory.find((entry) => entry.profile === 'unfinished')
      ?.accounts,
    'unavailable',
  );
  assert.deepEqual(snapshot.parties, []);
  assert.equal(
    snapshot.servers.find((server) => server.id === 'unfinished')?.trust.status,
    'unknown',
  );
  assert.equal(
    snapshot.notifications.some((note) => note.id.includes('unfinished')),
    false,
  );
  assert.equal(
    snapshot.servers.find((server) => server.id === 'unfinished')?.passiveStatus
      .status,
    'loading',
  );
  current = false;
  gate.resolve(catalog);
  await assert.rejects(pending, /retired/);
});

async function observePartial(
  response: CatalogDto,
  previous?: AgentSnapshot,
  nowSeconds = 2,
): Promise<AgentSnapshot> {
  const gate = deferred<CatalogDto>();
  const published = deferred<AgentSnapshot>();
  let current = true;
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    fixtureSnapshot: undefined,
    listCatalog: (publish) => {
      publish!(response);
      return gate.promise;
    },
  };
  const pending = loadSnapshot(
    bridge,
    previous,
    nowSeconds,
    published.resolve,
    () => current,
  );
  try {
    return await Promise.race([
      published.promise,
      new Promise<never>((_, reject) =>
        setTimeout(() => reject(new Error('no partial publication')), 500),
      ),
    ]);
  } finally {
    current = false;
    gate.resolve(response);
    await assert.rejects(pending, /retired/);
  }
}

async function pendingCatalog() {
  const bridge = mockBridge(FIXTURE);
  const previous = await loadSnapshot(bridge, FIXTURE, 1);
  const store = previous.stores.find((store) => store.id === 'acct:personal')!;
  const server = previous.servers.find((server) => server.id === store.server)!;
  const catalog = await bridge.listCatalog();
  const response: CatalogDto = {
    ...catalog,
    fullItemReads: [],
    items: [],
    localMetadata: {
      accounts: [],
      profiles: catalog.profiles.map((profile) => ({
        profile,
        configuredProbe: previous.servers.find(
          (server) => server.id === profile,
        )!.configuredProbe,
        label: null,
        status: null,
        error: null,
      })),
    },
  };
  return { previous, response, store, server };
}

const timeoutFailure = {
  code: 'deadline-exceeded',
  message: 'Timed out',
  retryable: true,
  fatal: false,
  ambiguous: false,
};

test('pending refresh retains accepted facts and metadata without completing freshness', async () => {
  const { previous, response, store, server } = await pendingCatalog();
  const first = await observePartial(response);
  assert.equal(
    first.servers.find((entry) => entry.id === server.id)!.trust.status,
    'unknown',
  );
  assert.equal(
    first.servers.find((entry) => entry.id === server.id)!.passiveStatus.status,
    'loading',
  );
  assert.equal(
    first.storeInventory.find((entry) => entry.store === store.id)!.status,
    'loading',
  );
  assert.equal(
    first.catalogFreshness?.profiles[server.id].lastSuccessAt,
    undefined,
  );
  const refreshed = await observePartial(response, previous);
  assert.equal(
    storeAvailability(refreshed, store, { nowSeconds: 2 }).available,
    true,
  );
  assert.ok(refreshed.items.some((item) => item.store === store.id));
  assert.equal(
    refreshed.catalogFreshness?.profiles[server.id].lastSuccessAt,
    1,
  );
  assert.equal(refreshed.catalogFreshness?.stores[store.id].lastSuccessAt, 1);
  assert.equal(
    refreshed.catalogFreshness?.profiles[server.id].refreshing,
    true,
  );
});

test('failed status invalidates prior authorization rather than remaining pending', async () => {
  const { previous, response, store } = await pendingCatalog();
  response.localMetadata!.profiles.find(
    (entry) => entry.profile === store.server,
  )!.error = timeoutFailure;
  const snapshot = await observePartial(response, previous);
  assert.deepEqual(storeAvailability(snapshot, store, { nowSeconds: 2 }), {
    available: false,
    reason: 'server-status-unavailable',
  });
  assert.equal(
    snapshot.catalogFreshness?.profiles[store.server].lastSuccessAt,
    1,
  );
  assert.deepEqual(
    snapshot.catalogFreshness?.profiles[store.server].error,
    timeoutFailure,
  );
});

test('pending KV denial does not disable independently accepted chat facts', async () => {
  const { previous, response, store, server } = await pendingCatalog();
  response.failures.push({
    scope: 'store',
    profile: server.id,
    store: store.id,
    error: {
      ...timeoutFailure,
      code: 'capability-denied',
      details: { capability: 'kv' },
    },
  });
  const snapshot = await observePartial(response, previous);
  const accepted = snapshot.servers.find((entry) => entry.id === server.id)!;
  assert.equal(
    storeAvailability(snapshot, store, { nowSeconds: 2 }).available,
    false,
  );
  assert.equal(
    serverCapabilityAvailability(snapshot, accepted, ['chat'], {
      nowSeconds: 2,
    }).available,
    true,
  );
  assert.ok(snapshot.items.some((item) => item.store === store.id));
});

test('changed configured identity cannot retain previous accepted facts', async () => {
  const { previous, response, store } = await pendingCatalog();
  response.localMetadata!.profiles.find(
    (entry) => entry.profile === store.server,
  )!.configuredProbe = 'different.example';
  const snapshot = await observePartial(response, previous);
  assert.equal(
    storeAvailability(snapshot, store, { nowSeconds: 2 }).available,
    false,
  );
  assert.equal(
    snapshot.items.some((item) => item.store === store.id),
    false,
  );
  assert.equal(
    snapshot.storeInventory.find((entry) => entry.store === store.id)!.status,
    'loading',
  );
});

test('retained lease facts cannot reopen after observed expiry and clock rollback', async () => {
  const { previous, response, store, server } = await pendingCatalog();
  const leased: AgentSnapshot = {
    ...previous,
    servers: previous.servers.map((entry) =>
      entry.id === server.id
        ? {
            ...entry,
            compatibility: {
              status: 'required',
              expiresAt: 10,
              capabilities: ['kv'],
            },
          }
        : entry,
    ),
  };
  const expired = await observePartial(response, leased, 11);
  const rollback = await observePartial(response, expired, 2);
  assert.deepEqual(storeAvailability(rollback, store, { nowSeconds: 2 }), {
    available: false,
    reason: 'check-in-expired',
  });
});

test('refresh failures retain last success without changing authorization facts', async () => {
  const { previous, store, server } = await pendingCatalog();
  const started = markCatalogRefresh(previous, [server.id], 2);
  const failed = failCatalogRefresh(started, [server.id], timeoutFailure, 3);
  assert.equal(failed.catalogFreshness?.profiles[server.id].lastSuccessAt, 1);
  assert.equal(failed.catalogFreshness?.stores[store.id].lastSuccessAt, 1);
  assert.equal(failed.catalogFreshness?.profiles[server.id].lastAttemptAt, 3);
  assert.equal(failed.catalogFreshness?.profiles[server.id].refreshing, false);
  assert.strictEqual(failed.servers, previous.servers);
});
