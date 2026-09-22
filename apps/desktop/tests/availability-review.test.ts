import assert from 'node:assert/strict';
import test from 'node:test';
import { PROTOCOL_CAPABILITIES } from '../src/model/types';

import { FIXTURE } from '../src/fixture';
import {
  profileInventoryComplete,
  storeAvailability,
} from '../src/model/lease';
import type { Server, AgentSnapshot } from '../src/model/types';

const error = {
  code: 'unsupported-schema',
  message: 'Display wording must not determine access.',
  retryable: false,
  ambiguous: false,
  fatal: true,
};

function snapshotWithServer(change: Partial<Server>): AgentSnapshot {
  const store = FIXTURE.stores[0];
  return {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server) =>
      server.profileName === store.server ? { ...server, ...change } : server,
    ),
  };
}

test('unknown empty inventory is not evidence that accounts disappeared', () => {
  const empty: AgentSnapshot = {
    ...FIXTURE,
    servers: [],
    stores: [],
    accounts: [],
    catalogProfiles: [],
    profileInventory: [],
    profileInventoryStatus: 'unavailable',
  };
  assert.equal(profileInventoryComplete(empty, 'accounts'), false);
  assert.equal(profileInventoryComplete(empty, 'teams'), false);
  assert.equal(
    profileInventoryComplete(
      { ...empty, profileInventoryStatus: 'complete' },
      'accounts',
    ),
    true,
  );
});

test('missing inventory for a catalog profile remains incomplete', () => {
  const snapshot: AgentSnapshot = { ...FIXTURE, profileInventory: [] };
  assert.equal(profileInventoryComplete(snapshot, 'accounts'), false);
  assert.equal(profileInventoryComplete(snapshot, 'teams'), false);
});

test('store schema restriction takes priority over lease expiry without blocking a neighbor', () => {
  const store = FIXTURE.stores[0];
  const snapshot = snapshotWithServer({
    compatibility: {
      status: 'required',
      expiresAt: 10,
      capabilities: PROTOCOL_CAPABILITIES,
    },
  });
  const restricted: AgentSnapshot = {
    ...snapshot,
    storeInventory: snapshot.storeInventory.map((entry) =>
      entry.store === store.id
        ? { ...entry, restrictions: [{ kind: 'schema-incompatible', error }] }
        : entry,
    ),
  };
  assert.deepEqual(storeAvailability(restricted, store, { nowSeconds: 10 }), {
    available: false,
    reason: 'schema-incompatible',
  });
  const neighbor = FIXTURE.stores.find(
    (entry) => entry.server !== store.server,
  );
  assert.ok(neighbor);
  assert.deepEqual(storeAvailability(restricted, neighbor, { nowSeconds: 0 }), {
    available: true,
  });
});

test('expiry is exact and an observed expired lease stays closed after a backward clock jump', () => {
  const store = FIXTURE.stores[0];
  const snapshot = snapshotWithServer({
    compatibility: {
      status: 'required',
      expiresAt: 10,
      capabilities: PROTOCOL_CAPABILITIES,
    },
  });
  assert.deepEqual(storeAvailability(snapshot, store, { nowSeconds: 9 }), {
    available: true,
  });
  assert.deepEqual(storeAvailability(snapshot, store, { nowSeconds: 10 }), {
    available: false,
    reason: 'check-in-expired',
  });
  const observed: AgentSnapshot = {
    ...snapshot,
    observedExpiredLeases: [{ profile: store.server, expiresAt: 10 }],
  };
  assert.deepEqual(storeAvailability(observed, store, { nowSeconds: 9 }), {
    available: false,
    reason: 'check-in-expired',
  });
});

test('a store restriction does not block another store on the same profile', () => {
  const store = FIXTURE.stores.find((entry) => entry.kind === 'team');
  assert.ok(store);
  const neighbor = FIXTURE.stores.find(
    (entry) => entry.id !== store.id && entry.server === store.server,
  );
  assert.ok(neighbor);
  const snapshot: AgentSnapshot = {
    ...FIXTURE,
    storeInventory: FIXTURE.storeInventory.map((entry) =>
      entry.store === store.id
        ? { ...entry, restrictions: [{ kind: 'schema-incompatible', error }] }
        : entry,
    ),
  };
  assert.deepEqual(storeAvailability(snapshot, store, { nowSeconds: 0 }), {
    available: false,
    reason: 'schema-incompatible',
  });
  assert.deepEqual(storeAvailability(snapshot, neighbor, { nowSeconds: 0 }), {
    available: true,
  });
});

test('bootstrap blocks access even when optional selector arguments are omitted', () => {
  const snapshot: AgentSnapshot = {
    ...FIXTURE,
    agent: { state: 'bootstrap', step: 'different words' },
  };
  assert.deepEqual(storeAvailability(snapshot, FIXTURE.stores[0]), {
    available: false,
    reason: 'agent-unavailable',
  });
});

test('unavailable passive status does not imply no lease or a connection failure', () => {
  const snapshot = snapshotWithServer({
    passiveStatus: {
      status: 'failed',
      source: 'describe-server-status',
      error,
    },
    compatibility: { status: 'requirement-unknown', error },
  });
  assert.deepEqual(storeAvailability(snapshot, FIXTURE.stores[0]), {
    available: false,
    reason: 'server-status-unavailable',
  });
});
