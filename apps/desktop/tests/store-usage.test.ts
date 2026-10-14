import assert from 'node:assert/strict';
import test from 'node:test';
import type { Bridge, StoreUsage } from '../src/bridge';
import { decodeStoreUsage } from '../src/bridge';
import type { AgentSnapshot, Store } from '../src/model';
import { FIXTURE } from '../src/fixture';
import { MetadataRepository } from '../src/metadata-repository';
import { mockBridge } from '../src/mock-bridge';
import {
  formatStoredBytes,
  isStoreUsageQuery,
  storeContentVersion,
  storeUsageKey,
  storeUsageQuery,
  storeUsageSummary,
  storeUsageText,
} from '../src/store-usage';

const store = (id: string): Store => {
  const found = FIXTURE.stores.find((candidate) => candidate.id === id);
  assert.ok(found, id);
  return found;
};

test('stored sizes use binary units and never state a fraction of a quota', () => {
  assert.equal(formatStoredBytes(0), '0 B');
  assert.equal(formatStoredBytes(1023), '1023 B');
  assert.equal(formatStoredBytes(3_482), '3.4 KiB');
  assert.equal(formatStoredBytes(1_048_000), '1023 KiB');
  assert.equal(formatStoredBytes(1_048_570), '1.0 MiB');
  assert.equal(formatStoredBytes(2.3 * 2 ** 30), '2.3 GiB');
  assert.equal(formatStoredBytes(14 * 2 ** 30), '14 GiB');
  assert.equal(
    storeUsageSummary({ files: 1_204, bytes: 2.3 * 2 ** 30 }),
    'About 2.3 GiB stored (encrypted) · 1,204 files',
  );
  assert.equal(
    storeUsageSummary({ files: 1, bytes: 512 }),
    '512 B stored (encrypted) · 1 file',
  );
  assert.doesNotMatch(
    storeUsageSummary({ files: 3, bytes: 16 * 2 ** 30 }),
    /%| of /,
  );
});

test('the usage line keeps the last figure through a failed re-read', () => {
  const base = { fetching: false, invalidation: 0 };
  assert.equal(
    storeUsageText({ ...base, data: undefined, error: undefined }),
    'Reading storage used…',
  );
  assert.equal(
    storeUsageText({ ...base, data: undefined, error: new Error('busy') }),
    'Storage used unavailable.',
  );
  assert.equal(
    storeUsageText({
      ...base,
      data: { files: 2, bytes: 4096 },
      error: new Error('busy'),
    }),
    'About 4.0 KiB stored (encrypted) · 2 files',
  );
});

test('the usage decoder accepts only a file count and a byte total', () => {
  assert.deepEqual(decodeStoreUsage({ files: 2, bytes: 10 }), {
    files: 2,
    bytes: 10,
  });
  for (const value of [
    { files: 2 },
    { files: 2, bytes: -1 },
    { files: 2, bytes: 10, limit: 100 },
    { files: '2', bytes: 10 },
    null,
  ])
    assert.throws(() => decodeStoreUsage(value), JSON.stringify(value));
});

test('the listing digest changes when a file is added or rewritten, and not before the listing loads', () => {
  const personal = store('acct:personal');
  const before = storeContentVersion(FIXTURE, personal);
  assert.ok(before);
  const first = FIXTURE.items.find(
    (item) => item.store === personal.id && item.kind !== 'Folder',
  );
  assert.ok(first);
  const rewritten: AgentSnapshot = {
    ...FIXTURE,
    items: FIXTURE.items.map((item) =>
      item === first ? { ...item, version: item.version + 1 } : item,
    ),
  };
  const added: AgentSnapshot = {
    ...FIXTURE,
    items: [...FIXTURE.items, { ...first, path: '/added', direntId: 'ff' }],
  };
  const otherStore: AgentSnapshot = {
    ...FIXTURE,
    items: FIXTURE.items.map((item) =>
      item.store !== personal.id
        ? { ...item, version: item.version + 1 }
        : item,
    ),
  };
  assert.notEqual(storeContentVersion(rewritten, personal), before);
  assert.notEqual(storeContentVersion(added, personal), before);
  assert.equal(storeContentVersion(otherStore, personal), before);
  assert.equal(
    storeContentVersion(
      {
        ...FIXTURE,
        storeInventory: FIXTURE.storeInventory.map((entry) =>
          entry.store === personal.id
            ? { ...entry, status: 'unavailable' as const }
            : entry,
        ),
      },
      personal,
    ),
    undefined,
  );
});

test('a usage row is read once per store and is not re-read on the metadata cadence', async () => {
  let now = 0;
  const repository = new MetadataRepository(() => now);
  const reads: string[] = [];
  const base = mockBridge(FIXTURE);
  const bridge: Bridge = {
    ...base,
    storeUsage: (id) => {
      reads.push(id);
      return base.storeUsage(id);
    },
  };
  const personal = store('acct:personal');
  const query = storeUsageQuery(repository, bridge, personal);
  assert.ok(isStoreUsageQuery(storeUsageKey(personal)));
  assert.equal(query, storeUsageQuery(repository, bridge, { ...personal }));
  const release = query.subscribe(() => undefined);
  const usage: StoreUsage = await query.load();
  assert.ok(usage.files > 0 && usage.bytes > 0);
  now += 24 * 60 * 60_000;
  await query.load();
  await repository.reconcileSubscribed();
  assert.deepEqual(reads, [personal.id]);
  query.invalidate();
  await query.load();
  assert.deepEqual(reads, [personal.id, personal.id]);
  release();
});
