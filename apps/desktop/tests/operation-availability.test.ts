import assert from 'node:assert/strict';
import test from 'node:test';
import { FIXTURE } from '../src/fixture';
import {
  chatAvailable,
  storeAvailability,
  storeOperationAvailability,
  type AgentSnapshot,
  type ProtocolCapability,
} from '../src/model';

const team = FIXTURE.stores.find((store) => store.id === 'team:household')!;
const account = FIXTURE.stores.find((store) => store.id === 'acct:personal')!;
function withGrants(
  capabilities: readonly ProtocolCapability[],
): AgentSnapshot {
  return {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server) =>
      server.id === team.server
        ? {
            ...server,
            compatibility: { status: 'required', expiresAt: 200, capabilities },
          }
        : server,
    ),
  };
}
const now = { nowSeconds: 100 };

test('an unavailable catalog does not imply a stopped local agent', () => {
  const snapshot = withGrants(['kv']);
  assert.deepEqual(
    storeAvailability(snapshot, account, { ...now, catalogReady: false }),
    { available: false, reason: 'store-metadata-unavailable' },
  );
  assert.deepEqual(
    storeAvailability(snapshot, account, { ...now, agentReady: false }),
    { available: false, reason: 'agent-unavailable' },
  );
});

test('KV readiness is independent of chat and device administration', () => {
  const snapshot = withGrants(['chat', 'teams', 'device-administration']);
  snapshot.storeInventory = snapshot.storeInventory.map((entry) => ({
    ...entry,
    status: 'unavailable',
  }));
  assert.deepEqual(storeAvailability(snapshot, team, now), {
    available: false,
    reason: 'capability-unavailable',
    capability: 'kv',
  });
  assert.equal(chatAvailable(snapshot, team, now), true);
  assert.deepEqual(
    storeOperationAvailability(snapshot, account, 'devices', now),
    { available: true },
  );
});

test('team vaults require both teams and KV, personal vaults only KV', () => {
  const snapshot = withGrants(['kv']);
  assert.deepEqual(storeAvailability(snapshot, team, now), {
    available: false,
    reason: 'capability-unavailable',
    capability: 'teams',
  });
  assert.deepEqual(storeAvailability(snapshot, account, now), {
    available: true,
  });
});

test('incompatible artifacts never grant access even with a future expiry', () => {
  const snapshot = withGrants(['chat', 'kv']);
  snapshot.servers = snapshot.servers.map((server) =>
    server.id === team.server
      ? {
          ...server,
          compatibility: {
            status: 'incompatible',
            reason: 'drift',
            expiresAt: 200,
          },
        }
      : server,
  );
  assert.deepEqual(storeAvailability(snapshot, account, now), {
    available: false,
    reason: 'compatibility-incompatible',
  });
  assert.equal(chatAvailable(snapshot, team, now), false);
  assert.equal(snapshot.servers[0].trust.status, 'verified');
});

test('expiry changes permission without changing advertised chat support', () => {
  const snapshot = withGrants(['chat']);
  assert.equal(chatAvailable(snapshot, team, { nowSeconds: 199 }), true);
  assert.deepEqual(
    storeOperationAvailability(snapshot, team, 'chat', { nowSeconds: 200 }),
    { available: false, reason: 'check-in-expired' },
  );
  assert.equal(snapshot.servers[0].capabilities.chat, true);
});

test('a scoped KV restriction does not become a chat restriction or a trust failure', () => {
  const snapshot = withGrants(['chat', 'kv', 'teams']);
  const error = {
    code: 'capability-denied',
    message: 'KV unavailable',
    retryable: false,
    ambiguous: false,
    fatal: false,
  };
  snapshot.servers = snapshot.servers.map((server) =>
    server.id === team.server
      ? {
          ...server,
          restrictions: [
            { kind: 'capability-denied', capability: 'kv', error },
          ],
        }
      : server,
  );
  assert.equal(storeAvailability(snapshot, account, now).available, false);
  assert.equal(chatAvailable(snapshot, team, now), true);
});

test('unknown and unsupported chat service facts are distinct from permission', () => {
  const granted = withGrants(['chat']);
  for (const chat of [null, false] as const) {
    const snapshot = {
      ...granted,
      servers: granted.servers.map((server) =>
        server.id === team.server
          ? { ...server, capabilities: { chat } }
          : server,
      ),
    };
    assert.deepEqual(storeOperationAvailability(snapshot, team, 'chat', now), {
      available: false,
      reason: chat === null ? 'server-status-unavailable' : 'chat-unsupported',
    });
  }
});

test('federation requires both grants and does not inherit a KV requirement', () => {
  assert.deepEqual(
    storeOperationAvailability(
      withGrants(['teams', 'federation']),
      team,
      'federation',
      now,
    ),
    { available: true },
  );
  assert.deepEqual(
    storeOperationAvailability(withGrants(['teams']), team, 'federation', now),
    {
      available: false,
      reason: 'capability-unavailable',
      capability: 'federation',
    },
  );
});

test('unknown metadata never becomes permission to use a cached store binding', () => {
  const snapshot = withGrants(['chat']);
  snapshot.profileInventory = [];
  snapshot.storeInventory = [];
  assert.deepEqual(storeOperationAvailability(snapshot, team, 'chat', now), {
    available: false,
    reason: 'store-metadata-unavailable',
  });
});

test('an explicit store schema restriction outranks an unknown server observation', () => {
  const snapshot = withGrants(['chat']);
  const error = {
    code: 'unsupported-schema',
    message: 'Unavailable',
    retryable: false,
    ambiguous: false,
    fatal: true,
  };
  snapshot.servers = snapshot.servers.map((server) =>
    server.id === team.server
      ? {
          ...server,
          trust: { status: 'unknown' },
          passiveStatus: { status: 'loading' },
        }
      : server,
  );
  snapshot.storeInventory = snapshot.storeInventory.map((entry) =>
    entry.store === team.id
      ? { ...entry, restrictions: [{ kind: 'schema-incompatible', error }] }
      : entry,
  );
  assert.deepEqual(storeOperationAvailability(snapshot, team, 'chat', now), {
    available: false,
    reason: 'schema-incompatible',
  });
});

test('schema, import, and trust restrictions remain common prerequisites', () => {
  const error = {
    code: 'unsupported-schema',
    message: 'Unavailable',
    retryable: false,
    ambiguous: false,
    fatal: true,
  };
  for (const kind of [
    'schema-incompatible',
    'import-verification-required',
  ] as const) {
    const snapshot = withGrants(['chat', 'device-administration']);
    snapshot.servers = snapshot.servers.map((server) =>
      server.id === team.server
        ? { ...server, restrictions: [{ kind, error }] }
        : server,
    );
    assert.equal(chatAvailable(snapshot, team, now), false);
    assert.equal(
      storeOperationAvailability(snapshot, account, 'devices', now).available,
      false,
    );
  }
});
