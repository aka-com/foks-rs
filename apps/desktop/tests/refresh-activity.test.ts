import assert from 'node:assert/strict';
import test from 'node:test';
import {
  RefreshActivities,
  refreshActivitiesFor,
} from '../src/refresh-activity';
import { loadSnapshot, type Bridge } from '../src/bridge';
import { FIXTURE } from '../src/fixture';
import { mockBridge } from '../src/mock-bridge';

const tick = () => new Promise<void>((resolve) => setImmediate(resolve));
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

test('retiring an owner retains its child work without clearing a newer operation', () => {
  const activities = new RefreshActivities();
  let current = true;
  const old = activities.begin('Loading catalog', () => current);
  const child = old.child('Loading team rosters');
  current = false;
  old.finish();
  const next = activities.begin('Waiting for current catalog reads');
  assert.equal(activities.getSnapshot().length, 2);
  assert.equal(activities.getSnapshot()[0].isCurrent(), false);
  assert.deepEqual(activities.getSnapshot()[0].phases, [
    'Loading team rosters',
  ]);
  child.finish();
  old.finish();
  child.update('Late callback');
  assert.deepEqual(
    activities.getSnapshot().map((entry) => entry.phases),
    [['Waiting for current catalog reads']],
  );
  next.finish();
  assert.deepEqual(activities.getSnapshot(), []);
});

test('catalog work stays observable after retirement until the response settles', async () => {
  const base = mockBridge(FIXTURE);
  const response = await base.listCatalog();
  const pending = deferred<typeof response>();
  const bridge: Bridge = { ...base, listCatalog: () => pending.promise };
  const activities = refreshActivitiesFor(bridge);
  let current = true;
  const read = loadSnapshot(bridge, FIXTURE, 1, undefined, () => current);
  const rejected = assert.rejects(read, { code: 'catalog-read-retired' });
  await tick();
  assert.deepEqual(activities.getSnapshot()[0].phases, ['Loading catalog']);
  current = false;
  assert.equal(activities.getSnapshot()[0].isCurrent(), false);
  pending.resolve(response);
  await rejected;
  assert.deepEqual(activities.getSnapshot(), []);
});

test('a failed projection retains other outstanding roster reads', async () => {
  const base = mockBridge(FIXTURE);
  const first = deferred<void>();
  const others = deferred<void>();
  let started = 0;
  const bridge: Bridge = {
    ...base,
    listGroupDetails: async (store) => {
      const gate = started++ === 0 ? first : others;
      await gate.promise;
      return base.listGroupDetails(store);
    },
  };
  const activities = refreshActivitiesFor(bridge);
  const read = loadSnapshot(bridge, FIXTURE, 1, undefined, () => true, true);
  const rejected = assert.rejects(read, { code: 'response-binding' });
  for (let i = 0; i < 20 && started < 2; i++) await tick();
  assert.ok(started >= 2);
  assert.ok(
    activities.getSnapshot()[0].phases.includes('Loading team rosters'),
  );
  first.reject({
    code: 'response-binding',
    message: 'Wrong session',
    retryable: false,
    fatal: true,
    ambiguous: false,
  });
  await rejected;
  assert.equal(activities.getSnapshot().length, 1);
  assert.deepEqual(activities.getSnapshot()[0].phases, [
    'Loading team rosters',
  ]);
  others.resolve();
  for (let i = 0; i < 20 && activities.getSnapshot().length; i++) await tick();
  assert.deepEqual(activities.getSnapshot(), []);
});

test('failed and synchronous-throwing auxiliary work clears its activity', async () => {
  const activities = new RefreshActivities();
  await assert.rejects(
    activities.run('Loading application information', () => {
      throw new Error('failed');
    }),
    /failed/,
  );
  assert.deepEqual(activities.getSnapshot(), []);
});
