import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import {
  decodeCompatibility,
  decodeServerStatus,
  decodeServers,
  discoverUnboundTeams,
  loadSnapshot,
  type Bridge,
} from '../src/bridge';
import { FIXTURE } from '../src/fixture';
import { mockBridge } from '../src/mock-bridge';
import {
  PROTOCOL_CAPABILITIES,
  serverAvailability,
  serverFactAvailability,
  storeAvailability,
  type Server,
  type ProtocolCapability,
} from '../src/model';

const failure = {
  code: 'deadline-exceeded',
  message: 'Timed out',
  retryable: true,
  ambiguous: false,
  fatal: false,
};

async function harness() {
  const base = mockBridge(FIXTURE);
  const source = await base.listCatalog();
  const store = FIXTURE.stores.find((entry) => entry.id === 'acct:personal')!;
  const server = FIXTURE.servers.find((entry) => entry.id === store.server)!;
  const catalog = {
    ...source,
    profiles: [server.id],
    stores: [store],
    knownStores: [store],
    inventory: [
      { profile: server.id, accountsComplete: true, teamsComplete: true },
    ],
    items: [],
    failures: [],
    blockedProfiles: [],
  };
  const bridge: Bridge = {
    ...base,
    native: true,
    listCatalog: async () => catalog,
    listServers: async () =>
      decodeServers([
        {
          id: server.id,
          name: server.name,
          label: server.label,
          configured_probe: server.configuredProbe,
          accounts: server.accounts,
        },
      ]),
    listAccounts: async () =>
      FIXTURE.accounts.filter((entry) => entry.store === store.id),
  };
  return { bridge, catalog, store, server };
}

test('startup discovery skips profiles without the teams capability', async () => {
  const { bridge, server, store } = await harness();
  const snapshot = {
    ...FIXTURE,
    servers: [
      {
        ...server,
        compatibility: {
          status: 'required' as const,
          expiresAt: 4e9,
          capabilities: ['kv' as const],
        },
      },
    ],
    stores: [store],
    accounts: FIXTURE.accounts.filter((entry) => entry.store === store.id),
    catalogProfiles: [server.id],
  };
  let calls = 0;
  assert.equal(
    await discoverUnboundTeams(
      {
        ...bridge,
        discoverGroups: async () => {
          calls++;
          throw new Error('must not dispatch');
        },
      },
      snapshot,
    ),
    false,
  );
  assert.equal(calls, 0);
});

test('a failed status observation is not evidence of an absent saved identity', async () => {
  const { bridge, store } = await harness();
  const snapshot = await loadSnapshot(
    {
      ...bridge,
      describeServerStatus: async () => {
        throw failure;
      },
    },
    FIXTURE,
  );
  assert.deepEqual(storeAvailability(snapshot, store), {
    available: false,
    reason: 'server-status-unavailable',
  });
  assert.notEqual(snapshot.servers[0].trust.status, 'unprobed');
  assert.equal(
    snapshot.notifications.some((entry) =>
      entry.id.startsWith('verification-required'),
    ),
    false,
  );
});

test('catalog trust blocking remains authoritative when listServers contains only metadata', async () => {
  const { bridge, catalog, server, store } = await harness();
  let statusCalls = 0;
  const snapshot = await loadSnapshot(
    {
      ...bridge,
      listCatalog: async () => ({ ...catalog, blockedProfiles: [server.id] }),
      describeServerStatus: async () => {
        statusCalls++;
        throw failure;
      },
    },
    FIXTURE,
  );
  assert.equal(statusCalls, 0);
  assert.equal(snapshot.servers[0].trust.status, 'blocked');
  assert.equal(snapshot.servers[0].services.chat, null);
  assert.deepEqual(storeAvailability(snapshot, store), {
    available: false,
    reason: 'verification-failed',
  });
});

test('schema failures are not hidden behind an unobserved identity', async () => {
  const { bridge, store } = await harness();
  const snapshot = await loadSnapshot(
    {
      ...bridge,
      describeServerStatus: async () => {
        throw { ...failure, code: 'unsupported-schema' };
      },
    },
    FIXTURE,
  );
  assert.deepEqual(storeAvailability(snapshot, store), {
    available: false,
    reason: 'schema-incompatible',
  });
});

test('store identity does not imply a successfully loaded item inventory', async () => {
  const { bridge, catalog, store } = await harness();
  const previous = {
    ...(await loadSnapshot(bridge, FIXTURE, 1)),
    items: FIXTURE.items.filter((item) => item.store === store.id),
  };
  const snapshot = await loadSnapshot(
    {
      ...bridge,
      listCatalog: async () => ({
        ...catalog,
        failures: [
          {
            scope: 'store',
            profile: store.server,
            store: store.id,
            error: failure,
          },
        ],
      }),
    },
    previous,
    2,
  );
  assert.deepEqual(storeAvailability(snapshot, store), {
    available: false,
    reason: 'vault-unavailable',
  });
  assert.deepEqual(snapshot.storeInventory[0].error, failure);
  assert.ok(snapshot.items.some((item) => item.store === store.id));
  assert.equal(
    snapshot.items.some((item) => item.value !== undefined),
    false,
  );
  assert.equal(snapshot.catalogFreshness?.stores[store.id].lastSuccessAt, 1);
  assert.equal(
    serverAvailability(snapshot, snapshot.servers[0]).available,
    true,
  );
});

test('successful complete empty listing clears historical item metadata', async () => {
  const { bridge, store } = await harness();
  assert.ok(FIXTURE.items.some((item) => item.store === store.id));
  const snapshot = await loadSnapshot(bridge, FIXTURE, 1);
  assert.equal(
    snapshot.items.some((item) => item.store === store.id),
    false,
  );
  assert.equal(snapshot.storeInventory[0].status, 'available');
  assert.equal(snapshot.catalogFreshness?.stores[store.id].lastSuccessAt, 1);
});

test('compatibility restrictions retain their scope without changing server trust', async () => {
  const { bridge, catalog, server } = await harness();
  const denied = {
    ...failure,
    code: 'capability-denied',
    details: { capability: 'kv' },
  };
  const snapshot = await loadSnapshot(
    {
      ...bridge,
      listCatalog: async () => ({
        ...catalog,
        failures: [
          {
            scope: 'profile',
            profile: server.id,
            source: 'KV catalog',
            error: denied,
          },
        ],
      }),
    },
    FIXTURE,
  );
  assert.equal(snapshot.servers[0].trust.status, 'verified');
  assert.equal(snapshot.servers[0].restrictions[0].kind, 'capability-denied');
  assert.equal(
    snapshot.notifications.some((entry) =>
      entry.id.startsWith('verification-failed'),
    ),
    false,
  );
});

test('shared wire contract separates metadata, service support, and compatibility grants', async () => {
  const fixture = JSON.parse(
    await readFile(
      new URL('../src-tauri/wire-contract.json', import.meta.url),
      'utf8',
    ),
  ) as {
    configuredServer: Record<string, unknown>;
    protocolCapabilities: string[];
    serverStatusRequiredFields: string[];
    serverStatus: Record<string, unknown>;
    compatibilityCases: {
      name: string;
      wire: unknown;
      decoded?: unknown;
      nowSeconds: number;
      granted: string[];
    }[];
  };
  assert.deepEqual(PROTOCOL_CAPABILITIES, fixture.protocolCapabilities);
  const [metadata] = decodeServers([fixture.configuredServer]);
  assert.ok(metadata);
  assert.deepEqual(metadata, {
    id: 'work',
    name: 'work',
    label: 'Work',
    configuredProbe: 'foks.example',
    accounts: ['personal'],
    host_id: null,
    chain: null,
    epoch: null,
    trust: { status: 'unknown' },
    compatibility: {
      status: 'requirement-unknown',
      error: {
        code: 'catalog-loading',
        message: 'Server facts have not been loaded.',
        retryable: false,
        ambiguous: false,
        fatal: false,
      },
    },
    passiveStatus: { status: 'loading' },
    connectivity: { status: 'unknown' },
    services: { chat: null },
    restrictions: [],
  });
  assert.deepEqual(serverFactAvailability(metadata, []), {
    available: false,
    reason: 'loading',
  });
  for (const field of Object.keys(fixture.configuredServer)) {
    const incomplete = { ...fixture.configuredServer };
    delete incomplete[field];
    assert.throws(() => decodeServers([incomplete]), field);
  }
  for (const field of [
    'host_id',
    'chain',
    'epoch',
    'lease',
    'state',
    'chat_available',
  ]) {
    assert.throws(
      () => decodeServers([{ ...fixture.configuredServer, [field]: null }]),
      /unexpected server metadata fields/,
    );
  }
  for (const capability of PROTOCOL_CAPABILITIES) {
    assert.deepEqual(
      decodeCompatibility({
        status: 'validated',
        expires_at: 200,
        capabilities: [capability],
      }),
      { status: 'required', expiresAt: 200, capabilities: [capability] },
    );
  }
  for (const entry of fixture.compatibilityCases) {
    const wire = { ...fixture.serverStatus, compatibility: entry.wire };
    if (entry.decoded === undefined) {
      assert.throws(() => decodeCompatibility(entry.wire), entry.name);
      assert.throws(() => decodeServerStatus(wire), entry.name);
      continue;
    }
    const status = decodeServerStatus(wire);
    assert.deepEqual(status.compatibility, entry.decoded, entry.name);
    assert.equal(status.chatSupported, true, entry.name);
    assert.ok(status.host, entry.name);
    const server: Server = {
      ...metadata,
      host_id: status.host.hostId,
      chain: status.host.chain,
      epoch: status.host.epoch,
      trust: { status: 'verified' },
      passiveStatus: { status: 'available', source: 'signed-server-status' },
      compatibility: status.compatibility,
      services: { chat: status.chatSupported },
    };
    PROTOCOL_CAPABILITIES.forEach((capability: ProtocolCapability) => {
      assert.equal(
        serverFactAvailability(server, [], { nowSeconds: entry.nowSeconds }, [
          capability,
        ]).available,
        entry.granted.includes(capability),
        `${entry.name}: ${capability}`,
      );
    });
    assert.equal(server.services.chat, true, entry.name);
    assert.equal(server.trust.status, 'verified', entry.name);
  }
  for (const field of fixture.serverStatusRequiredFields) {
    for (const noHost of [false, true]) {
      const incomplete = { ...fixture.serverStatus };
      if (noHost) {
        incomplete.host = null;
        incomplete.chatSupported = null;
      }
      delete incomplete[field];
      assert.throws(() => decodeServerStatus(incomplete), field);
    }
  }
});

test('compatibility decoding preserves rejected outcomes and validates grant sets', () => {
  assert.deepEqual(
    decodeCompatibility({
      status: 'incompatible',
      expires_at: 200,
      reason: 'drift',
    }),
    { status: 'incompatible', expiresAt: 200, reason: 'drift' },
  );
  for (const capabilities of [[], ['invented'], ['kv', 'kv'], ['probe']]) {
    assert.throws(() =>
      decodeCompatibility({
        status: 'validated',
        expires_at: 200,
        capabilities,
      }),
    );
  }
  assert.deepEqual(
    decodeCompatibility({
      status: 'validated',
      expires_at: 200,
      capabilities: ['chat'],
    }),
    { status: 'required', expiresAt: 200, capabilities: ['chat'] },
  );
  assert.throws(
    () =>
      decodeServerStatus({
        profile: 'p',
        configuredProbe: 'p',
        host: null,
        chatSupported: true,
        compatibility: { status: 'not-required' },
      }),
    /inconsistent host support/,
  );
});

test('mock service support stays unknown until a new server is checked', async () => {
  const bridge = mockBridge(FIXTURE);
  await bridge.addServer('new-server', 'new.example');
  const before = await bridge.describeServerStatus('new-server');
  assert.equal(before.host, null);
  assert.equal(before.chatSupported, null);
  await bridge.checkServer('new-server');
  const after = await bridge.describeServerStatus('new-server');
  assert.ok(after.host);
  assert.equal(after.chatSupported, false);
});

test('a successful observation of no host remains verification-required', async () => {
  const { bridge, store } = await harness();
  const describe = (profile: string) => bridge.describeServerStatus(profile);
  const snapshot = await loadSnapshot(
    {
      ...bridge,
      describeServerStatus: async (profile) => ({
        ...(await describe(profile)),
        host: null,
        chatSupported: null,
      }),
    },
    FIXTURE,
  );
  assert.deepEqual(storeAvailability(snapshot, store), {
    available: false,
    reason: 'verification-required',
  });
});
