import assert from 'node:assert/strict';
import test, { mock } from 'node:test';
import type { Channel } from '@tauri-apps/api/core';
import { vaultCommands } from '../src/bridge/commands-vault';
import {
  enqueueProfileWork,
  loadSnapshot,
  loadProfileSnapshot,
  type Bridge,
  type CatalogDto,
} from '../src/bridge';
import { FIXTURE } from '../src/fixture';
import { mockBridge } from '../src/mock-bridge';
import {
  profileInventoryComplete,
  serverCapabilityAvailability,
  storeAvailability,
  type AgentSnapshot,
  type TeamStore,
} from '../src/model';
import { failCatalogRefresh, markCatalogRefresh } from '../src/catalog-state';
import { settle } from './lib/dom-harness';

const tick = () => new Promise<void>((done) => setImmediate(done));

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

test('valid final catalogs supersede failed partial projections and failed publication callbacks', async (t) => {
  t.mock.method(performance, 'now', () => 0);
  for (const publicationFailure of [false, true]) {
    const base = mockBridge(FIXTURE);
    const catalog = await base.listCatalog();
    const expected = await loadSnapshot(base, FIXTURE, 1);
    let publications = 0;
    let emit!: (value: CatalogDto) => void;
    const bridge: Bridge = {
      ...base,
      listCatalog: async (publish) => {
        emit = publish!;
        emit(
          publicationFailure
            ? catalog
            : {
                ...catalog,
                inventory: [...catalog.inventory, catalog.inventory[0]],
              },
        );
        await new Promise((resolve) => setImmediate(resolve));
        emit(catalog);
        await new Promise((resolve) => setImmediate(resolve));
        return catalog;
      },
    };
    const actual = await loadSnapshot(bridge, FIXTURE, 1, () => {
      publications++;
      if (publicationFailure && publications === 1)
        throw new Error('Projection consumer failed.');
    });
    assert.deepEqual(actual, expected);
    assert.equal(publications, publicationFailure ? 2 : 1);
    emit(catalog);
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(publications, publicationFailure ? 2 : 1);
  }
});

/**
 * A catalog read whose partials the test emits by hand, with each partial
 * labelled so the projection that reads it can be named.
 *
 * Every projection reads `failures` before its first await, so a getter there
 * records which partials were projected and which were dropped unread.
 */
function labelledPartials(catalog: CatalogDto) {
  const projected: string[] = [];
  const label = (name: string): CatalogDto => {
    const dto: CatalogDto = { ...catalog };
    Object.defineProperty(dto, 'failures', {
      enumerable: true,
      get() {
        if (projected.at(-1) !== name) projected.push(name);
        return catalog.failures;
      },
    });
    return dto;
  };
  return { projected, label };
}

test('several partials arriving in one tick produce one projection', async () => {
  const base = mockBridge(FIXTURE);
  const catalog = await base.listCatalog();
  const { projected, label } = labelledPartials(catalog);
  const gate = deferred<CatalogDto>();
  const started = deferred<void>();
  let emit!: (partial: CatalogDto) => void;
  const bridge: Bridge = {
    ...base,
    listCatalog: (onPartial) => {
      emit = onPartial!;
      started.resolve();
      return gate.promise;
    },
  };
  let publications = 0;
  const pending = loadSnapshot(bridge, FIXTURE, 1, () => {
    publications++;
  });
  await started.promise;
  emit(label('first'));
  emit(label('second'));
  emit(label('third'));
  await settle();
  // Only the newest partial is projected: each one carries the whole
  // accumulated catalog, so the two it superseded are dropped unread.
  assert.deepEqual(projected, ['third']);
  assert.equal(publications, 1);
  gate.resolve(catalog);
  assert.ok(await pending);
});

test('a partial arriving after the previous settled is still projected', async () => {
  const base = mockBridge(FIXTURE);
  const catalog = await base.listCatalog();
  const { projected, label } = labelledPartials(catalog);
  const gate = deferred<CatalogDto>();
  const started = deferred<void>();
  let emit!: (partial: CatalogDto) => void;
  const bridge: Bridge = {
    ...base,
    listCatalog: (onPartial) => {
      emit = onPartial!;
      started.resolve();
      return gate.promise;
    },
  };
  let publications = 0;
  const pending = loadSnapshot(bridge, FIXTURE, 1, () => {
    publications++;
  });
  await started.promise;
  emit(label('first'));
  await settle();
  assert.deepEqual(projected, ['first']);
  assert.equal(publications, 1);
  // The slot is empty again, so the next partial is not swallowed by the one
  // before it: first paint is never the only paint.
  emit(label('second'));
  await settle();
  assert.deepEqual(projected, ['first', 'second']);
  assert.equal(publications, 2);
  gate.resolve(catalog);
  assert.ok(await pending);
});

test('a terminal partial failure cannot be hidden by a valid final catalog', async () => {
  const base = mockBridge(FIXTURE);
  const catalog = await base.listCatalog();
  const failure = {
    code: 'response-binding',
    message: 'Wrong session.',
    retryable: false,
    fatal: true,
    ambiguous: false,
  };
  const bridge: Bridge = {
    ...base,
    listCatalog: async (publish) => {
      publish!(catalog);
      await new Promise((resolve) => setImmediate(resolve));
      return catalog;
    },
  };
  await assert.rejects(
    loadSnapshot(bridge, FIXTURE, 1, () => {
      throw failure;
    }),
    { code: 'response-binding' },
  );
});

test('terminal projection failures are settled before accepting an immediately returned final catalog', async () => {
  const base = mockBridge(FIXTURE);
  const catalog = await base.listCatalog();
  const bridge: Bridge = {
    ...base,
    listCatalog: async (publish) => {
      publish!({
        ...catalog,
        failures: [
          {
            scope: 'profile',
            profile: catalog.profiles[0],
            source: 'catalog',
            error: {
              code: 'agent-lost',
              message: 'Disconnected.',
              retryable: true,
              fatal: true,
              ambiguous: false,
            },
          },
        ],
      });
      return catalog;
    },
  };
  await assert.rejects(
    loadSnapshot(bridge, FIXTURE, 1, () => {}),
    { code: 'agent-lost' },
  );
});

test('final errors take precedence over obsolete recoverable partial errors', async () => {
  const base = mockBridge(FIXTURE);
  const catalog = await base.listCatalog();
  const bridge: Bridge = {
    ...base,
    listCatalog: async (publish) => {
      publish!({
        ...catalog,
        inventory: [...catalog.inventory, catalog.inventory[0]],
      });
      return catalog;
    },
    listAccounts: async () => {
      throw {
        code: 'operation-failed',
        message: 'Final read failed.',
        retryable: true,
        ambiguous: false,
        fatal: false,
      };
    },
  };
  await assert.rejects(
    loadSnapshot(bridge, FIXTURE, 1, () => {}),
    { code: 'operation-failed' },
  );
});

test('a final projection is not published after access retirement', async () => {
  const base = mockBridge(FIXTURE);
  let current = true;
  const bridge: Bridge = {
    ...base,
    listAccounts: async () => {
      current = false;
      return base.listAccounts();
    },
  };
  await assert.rejects(
    loadSnapshot(bridge, FIXTURE, 1, undefined, () => current),
    { code: 'catalog-read-retired' },
  );
});

for (const terminal of [false, true]) {
  test(`native catalog progress only retains terminal failures: ${terminal}`, async (t) => {
    const previous = Object.getOwnPropertyDescriptor(globalThis, 'window');
    const catalog: CatalogDto = {
      profiles: [],
      stores: [],
      knownStores: [],
      items: [],
      inventory: [],
      failures: [],
      blockedProfiles: [],
    };
    let channel!: Channel<unknown>;
    Object.defineProperty(globalThis, 'window', {
      configurable: true,
      value: {
        __TAURI_INTERNALS__: {
          transformCallback: () => 1,
          invoke: async (
            _command: string,
            args: { onPartial: Channel<unknown> },
          ) => {
            channel = args.onPartial;
            channel.onmessage(terminal ? catalog : { invalid: true });
            channel.onmessage(catalog);
            return catalog;
          },
        },
      },
    });
    t.after(() => {
      if (previous) Object.defineProperty(globalThis, 'window', previous);
      else delete (globalThis as { window?: Window }).window;
    });
    let publications = 0;
    const pending = vaultCommands.listCatalog(() => {
      publications++;
      if (terminal)
        throw {
          code: 'protocol',
          message: 'Invalid session.',
          retryable: false,
          fatal: true,
          ambiguous: false,
        };
    });
    if (terminal) await assert.rejects(pending, { code: 'protocol' });
    else assert.deepEqual(await pending, catalog);
    const accepted = publications;
    channel.onmessage(catalog);
    assert.equal(publications, accepted);
  });
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
  await assert.rejects(pending, {
    code: 'catalog-read-retired',
    fatal: false,
    ambiguous: false,
  });
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
    await assert.rejects(pending, {
      code: 'catalog-read-retired',
      fatal: false,
      ambiguous: false,
    });
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

test('a roster the native side could not decode degrades that team, not the whole catalog', async () => {
  const base = mockBridge(FIXTURE);
  const team = FIXTURE.stores.find(
    (store): store is TeamStore => store.kind === 'team',
  )!;
  const bridge: Bridge = {
    ...base,
    listGroupDetails: async (storeId) => {
      const details = await base.listGroupDetails(storeId);
      if (storeId !== team.id) return details;
      return {
        ...details,
        parties: {
          status: 'error',
          error: {
            code: 'invalid-response',
            message: 'group roster entries exceeded the response cap',
            retryable: false,
            fatal: false,
            ambiguous: false,
          },
        },
      };
    },
  };
  const snapshot = await loadSnapshot(bridge, FIXTURE, 1);
  const failure = snapshot.groupDetailFailures.find(
    (entry) => entry.store === team.id,
  );
  assert.equal(failure?.source, 'roster');
  assert.equal(failure?.code, 'invalid-response');
  assert.ok(snapshot.stores.some((store) => store.id === team.id));
  assert.ok(
    snapshot.parties.every((party) => party.store !== team.id),
    'no roster was kept for the team that failed',
  );
  assert.ok(
    snapshot.parties.some((party) => party.store !== team.id),
    'the other teams were read',
  );
});

test('a roster the profile queue cannot admit degrades that team, not the whole catalog', async () => {
  const base = mockBridge(FIXTURE);
  const team = FIXTURE.stores.find(
    (store): store is TeamStore => store.kind === 'team',
  )!;
  // Timers are mocked for the whole load: the fillers below never run, and
  // the admission deadline is reached by ticking, not by waiting a minute.
  mock.timers.enable({ apis: ['setTimeout'] });
  let filled = false;
  // The account read is the last read before the roster stage. Filling the
  // profile's queue there leaves the roster read waiting behind work that
  // never finishes, so it reaches its admission deadline.
  const bridge: Bridge = {
    ...base,
    listAccounts: async (generation) => {
      const accounts = await base.listAccounts(generation);
      for (let i = 0; i < 256; i++)
        void enqueueProfileWork(
          bridge,
          team.server,
          () => new Promise<never>(() => {}),
        ).catch(() => undefined);
      filled = true;
      return accounts;
    },
  };
  try {
    const pending = loadSnapshot(bridge, FIXTURE, 1);
    let settled = false;
    void pending.then(
      () => (settled = true),
      () => (settled = true),
    );
    for (let round = 0; round < 5_000 && !filled; round++) await tick();
    assert.ok(filled, 'the account read ran');
    // Let the roster stage submit its reads, then run out their wait.
    await tick();
    await tick();
    mock.timers.tick(60_000);
    for (let round = 0; round < 5_000 && !settled; round++) {
      await tick();
      mock.timers.tick(1_000);
    }
    assert.ok(settled, 'the load settled');
    const snapshot = await pending;
    const failure = snapshot.groupDetailFailures.find(
      (entry) => entry.store === team.id,
    );
    assert.equal(failure?.source, 'roster');
    assert.equal(failure?.code, 'profile-busy');
    assert.match(failure?.message ?? '', /deadline/);
    assert.ok(snapshot.stores.some((store) => store.id === team.id));
    assert.ok(
      snapshot.parties.every((party) => party.store !== team.id),
      'no roster was read for the refused team',
    );
  } finally {
    mock.timers.reset();
  }
});

test('initial and scoped catalog loads record elapsed duration', async (t) => {
  let now = 0;
  t.mock.method(performance, 'now', () => now);
  const base = mockBridge(FIXTURE);
  const bridge: Bridge = {
    ...base,
    listCatalog: async (...args) => {
      const result = await base.listCatalog(...args);
      now += 125;
      return result;
    },
    listProfileCatalog: async (...args) => {
      const result = await base.listProfileCatalog(...args);
      now += 75;
      return result;
    },
  };
  const initial = await loadSnapshot(bridge, FIXTURE, 1);
  assert.equal(initial.catalogFreshness?.attempt?.lastMilliseconds, 125);
  const successful = Object.entries(initial.catalogFreshness!.profiles).filter(
    ([, value]) => value.lastSuccessAt === 1 && !value.error,
  );
  assert.ok(successful.length);
  for (const [, value] of successful) assert.equal(value.lastMilliseconds, 125);
  const profile = successful[0][0];
  const scoped = await loadProfileSnapshot(bridge, profile, initial, 2);
  assert.equal(scoped.catalogFreshness?.profiles[profile].lastMilliseconds, 75);
  for (const [other, value] of successful) {
    if (other !== profile)
      assert.equal(
        scoped.catalogFreshness?.profiles[other].lastMilliseconds,
        value.lastMilliseconds,
      );
  }
});
