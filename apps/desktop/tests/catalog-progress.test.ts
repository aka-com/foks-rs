import assert from 'node:assert/strict';
import test from 'node:test';
import { loadSnapshot, type Bridge, type CatalogDto } from '../src/bridge';
import { FIXTURE } from '../src/fixture';
import { mockBridge } from '../src/mock-bridge';
import { profileInventoryComplete, type AgentSnapshot } from '../src/model';

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
    snapshot.servers.find((server) => server.id === 'unfinished')?.passiveStatus
      .status,
    'failed',
  );
  current = false;
  gate.resolve(catalog);
  await assert.rejects(pending, /retired/);
});
