import assert from 'node:assert/strict';
import test from 'node:test';
import { DeviceCache } from '../src/device-cache';
import { RetiredQueryError } from '../src/query-repository';
import type { Bridge, AccountDevice } from '../src/bridge';

function fixture() {
  let now = 0;
  const calls = {
    devices: [] as string[],
    backups: [] as string[],
    enrollments: [] as string[],
    hardware: 0,
  };
  const bridge = {
    listAccountDevices: async (store: string) => {
      calls.devices.push(store);
      return [{ id: store, name: 'Mac' }] as AccountDevice[];
    },
    listBackupEnrollments: async (store: string) => {
      calls.backups.push(store);
      return [];
    },
    listYubiAccounts: async (profile: string) => {
      calls.enrollments.push(profile);
      return [];
    },
    listYubiCards: async () => {
      calls.hardware++;
      throw new Error('Do not probe hardware');
    },
  } as unknown as Bridge;
  return {
    calls,
    bridge,
    clock: () => now,
    advance: () => {
      now += 60_001;
    },
  };
}

test('device metadata deduplicates concurrent reads and reuses fresh values without hardware', async () => {
  const f = fixture();
  const cache = new DeviceCache(f.bridge, f.clock);
  const [a, b] = await Promise.all([
    cache.load('p', 'a'),
    cache.load('p', 'a'),
  ]);
  assert.deepEqual(a, b);
  await cache.load('p', 'a');
  assert.deepEqual(f.calls, {
    devices: ['a'],
    backups: ['a'],
    enrollments: ['p'],
    hardware: 0,
  });
  assert.equal(cache.peek('p', 'a')?.devices[0].name, 'Mac');
});

test('accounts are isolated while enrollments are shared only within a profile', async () => {
  const f = fixture();
  const cache = new DeviceCache(f.bridge, f.clock);
  await Promise.all([cache.load('p', 'a'), cache.load('p', 'b')]);
  await cache.load('q', 'a');
  assert.deepEqual(f.calls.devices, ['a', 'b', 'a']);
  assert.deepEqual(f.calls.enrollments, ['p', 'q']);
  assert.equal(cache.peek('p', 'b')?.devices[0].id, 'b');
  assert.equal(cache.peek('q', 'b'), undefined);
});

test('expired metadata stays available during refresh; failed reads retry', async () => {
  const f = fixture();
  const cache = new DeviceCache(f.bridge, f.clock);
  await cache.load('p', 'a');
  f.advance();
  let reject!: (error: Error) => void;
  f.bridge.listAccountDevices = () =>
    new Promise((_resolve, no) => {
      reject = no;
    });
  const pending = cache.load('p', 'a');
  assert.ok(cache.peek('p', 'a'));
  await new Promise<void>((resolve) => setImmediate(resolve));
  reject(new Error('offline'));
  await assert.rejects(pending, /offline/);
  assert.ok(cache.peek('p', 'a'));
  f.bridge.listAccountDevices = async () => [];
  assert.deepEqual((await cache.load('p', 'a')).devices, []);
});

test('retiring a session clears metadata and late replies cannot refill it', async () => {
  const f = fixture();
  const cache = new DeviceCache(f.bridge, f.clock);
  let resolve!: (devices: AccountDevice[]) => void;
  f.bridge.listAccountDevices = () =>
    new Promise((yes) => {
      resolve = yes;
    });
  const pending = cache.load('p', 'a');
  await new Promise<void>((done) => setImmediate(done));
  cache.clear();
  resolve([]);
  await assert.rejects(pending, RetiredQueryError);
  assert.equal(cache.peek('p', 'a'), undefined);
  f.bridge.listAccountDevices = async () => [];
  await cache.load('p', 'a');
  assert.ok(cache.peek('p', 'a'), 'effect replay can reuse an emptied cache');
});
