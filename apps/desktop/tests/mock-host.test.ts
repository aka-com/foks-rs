import assert from 'node:assert/strict';
import test from 'node:test';
import { createBridge } from '../src/bridge/create';
import { createTransport } from '../src/bridge/transport';
import { FIXTURE } from '../src/fixture';
import { mockBridge } from '../src/mock-bridge';
import { mockCommand, mockHost } from '../src/mock/host';

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
