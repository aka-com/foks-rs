import assert from 'node:assert/strict';
import test from 'node:test';
import { decodeAccounts } from '../src/bridge/accounts-groups';
import { createBridge } from '../src/bridge/create';
import {
  decodeServerStatus,
  type ServerStatusSnapshot,
} from '../src/bridge/servers';
import { createTransport } from '../src/bridge/transport';
import { FIXTURE } from '../src/fixture';
import { mockBridge } from '../src/mock-bridge';
import { mockCommand, mockHost } from '../src/mock/host';
import { accountWire, serverStatusWire } from '../src/mock/wire';
import type { Account } from '../src/model/types';

/** A wire value as it crosses the IPC boundary. */
const overWire = (value: unknown): unknown =>
  JSON.parse(JSON.stringify(value)) as unknown;

test('the mock host answers through the bridge decoders and rejects what they reject', async () => {
  let response: unknown = {
    store: 'acct:personal',
    path: '/a',
    version: 1,
    value: 'text',
    extra: undefined,
  };
  const bridge = createBridge(
    createTransport(
      mockHost({
        read_item: mockCommand(() => response),
        create_folder: () => ({ applied: 'yes' }),
      }),
    ),
  );
  const request = { storeId: 'acct:personal', path: '/a', version: 1 };
  // Serialized as Tauri does, so an undefined field is not part of the reply.
  assert.deepEqual(await bridge.readItem(request), {
    store: 'acct:personal',
    path: '/a',
    version: 1,
    value: 'text',
  });
  response = { ...(response as object), version: '1' };
  await assert.rejects(bridge.readItem(request), {
    code: 'invalid-response',
    origin: 'response',
    ambiguous: false,
  });
  await assert.rejects(
    bridge.createFolder({ storeId: 'acct:personal', path: '/b' }),
    { code: 'invalid-response', origin: 'response', ambiguous: true },
  );
  await assert.rejects(bridge.storeUsage('acct:personal'), {
    code: 'invalid-request',
    origin: 'invoke',
    message: 'The mock host has no command store_usage.',
  });
});

test('a mock handler fault keeps its message through the transport', async () => {
  const bridge = createBridge(
    createTransport(
      mockHost({
        store_usage: () => {
          throw new Error('The handler failed.');
        },
      }),
    ),
  );
  await assert.rejects(bridge.storeUsage('acct:personal'), {
    origin: 'invoke',
    message: 'The handler failed.',
  });
});

test('the mock wire encoders invert the bridge decoders', () => {
  const accounts: Account[] = [
    {
      store: 'acct:personal',
      server: 'foks.app',
      alias: 'alice',
      username: 'alice',
    },
    {
      store: 'acct:work',
      server: 'work.example',
      localAlias: 'work',
      alias: 'alice@work',
      username: 'alice',
    },
  ];
  assert.deepEqual(
    decodeAccounts(overWire(accounts.map((account) => accountWire(account)))),
    accounts,
  );
  const host = {
    lookupName: 'foks.app',
    canonicalName: 'foks.app',
    hostId: '02' + 'ab'.repeat(32),
    chain: 3,
    epoch: 1,
  };
  const statuses: ServerStatusSnapshot[] = [
    {
      profile: 'foks.app',
      configuredEndpoint: 'foks.app:443',
      host,
      leaseRequired: false,
      leaseExpiresAt: null,
      compatibility: { status: 'not-required' },
      chatSupported: true,
    },
    {
      profile: 'foks.app',
      configuredEndpoint: 'foks.app:443',
      host: null,
      leaseRequired: true,
      leaseExpiresAt: null,
      compatibility: { status: 'required-unavailable' },
      chatSupported: null,
    },
    {
      profile: 'foks.app',
      configuredEndpoint: 'foks.app:443',
      host,
      leaseRequired: true,
      leaseExpiresAt: 1_900_000_000,
      compatibility: {
        status: 'required',
        expiresAt: 1_900_000_000,
        capabilities: ['signup', 'kv'],
      },
      chatSupported: false,
    },
    {
      profile: 'foks.app',
      configuredEndpoint: 'foks.app:443',
      host,
      leaseRequired: true,
      leaseExpiresAt: 1_900_000_000,
      compatibility: {
        status: 'incompatible',
        expiresAt: 1_900_000_000,
        reason: 'protocol-mismatch',
      },
      chatSupported: true,
    },
  ];
  for (const status of statuses)
    assert.deepEqual(
      decodeServerStatus(overWire(serverStatusWire(status))),
      status,
    );
});

test('mock vault failures carry the host codes and the transport normalization', async () => {
  const bridge = mockBridge(FIXTURE);
  const item = (await bridge.listCatalog()).items.find(
    (entry) => entry.path === '/logins/github.com',
  );
  assert.ok(item);
  await assert.rejects(
    bridge.readItem({
      storeId: item.store,
      path: item.path,
      version: item.version + 1,
    }),
    {
      code: 'item-version-mismatch',
      origin: 'invoke',
      retryable: true,
      ambiguous: false,
    },
  );
  await assert.rejects(
    bridge.readItem({ storeId: item.store, path: '/missing', version: 1 }),
    { code: 'item-not-found', origin: 'invoke', ambiguous: false },
  );
  await assert.rejects(
    bridge.editTextItem({
      storeId: item.store,
      path: item.path,
      version: item.version,
      direntId: 'stale',
      value: 'changed',
    }),
    { code: 'conflict', origin: 'invoke', ambiguous: false },
  );
});

test('mock password import progress arrives through the transport channel', async () => {
  const bridge = mockBridge(FIXTURE);
  const preview = await bridge.beginPasswordImport();
  assert.ok(preview);
  const progress: string[] = [];
  const outcomes = await bridge.commitPasswordImport(
    {
      importId: preview.id,
      entries: [0, 1],
      storeId: 'acct:personal',
      folder: '/imported',
    },
    (outcome) => progress.push(`${outcome.index}:${outcome.status}`),
  );
  assert.deepEqual(progress, ['0:created', '1:created']);
  assert.deepEqual(
    outcomes.map((outcome) => outcome.status),
    ['created', 'created'],
  );
});
