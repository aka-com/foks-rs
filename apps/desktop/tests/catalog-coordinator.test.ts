import assert from 'node:assert/strict';
import test from 'node:test';
import {
  CatalogCoordinator,
  CatalogReadRetiredError,
} from '../src/catalog-coordinator';

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

test('compound reads stay serial and concurrent invalidations share a trailing load', async () => {
  const reads = [deferred<number>(), deferred<number>()];
  const published: number[] = [];
  let calls = 0;
  const coordinator = new CatalogCoordinator(
    () => reads[calls++].promise,
    (value) => published.push(value),
  );
  const first = coordinator.refresh();
  await Promise.resolve();
  assert.equal(calls, 1);
  assert.equal(coordinator.refresh(), first);
  assert.equal(coordinator.refresh(true), first);
  assert.equal(coordinator.refresh(true), first);
  assert.equal(calls, 1);
  reads[0].resolve(1);
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(calls, 2);
  assert.deepEqual(published, []);
  reads[1].resolve(2);
  assert.equal(await first, 2);
  assert.deepEqual(published, [2]);
});

test('failed superseded read proceeds to the requested replacement without publication', async () => {
  const first = deferred<number>();
  let calls = 0;
  const published: number[] = [];
  const coordinator = new CatalogCoordinator(
    () => (++calls === 1 ? first.promise : Promise.resolve(2)),
    (value) => published.push(value),
  );
  const pending = coordinator.refresh();
  await Promise.resolve();
  void coordinator.refresh(true);
  first.reject(new Error('old catalog'));
  assert.equal(await pending, 2);
  assert.deepEqual(published, [2]);
});

test('access reset retires both scheduled and active replies', async () => {
  let calls = 0;
  const read = deferred<number>();
  const published: number[] = [];
  const coordinator = new CatalogCoordinator(
    () => {
      calls++;
      return read.promise;
    },
    (value) => published.push(value),
  );
  const scheduled = coordinator.refresh();
  coordinator.reset();
  await assert.rejects(scheduled, CatalogReadRetiredError);
  assert.equal(calls, 0);
  const active = coordinator.refresh();
  await Promise.resolve();
  coordinator.reset();
  read.resolve(1);
  await assert.rejects(active, CatalogReadRetiredError);
  assert.deepEqual(published, []);
});

test('recovery after access reset waits for old work before loading the new scope', async () => {
  const first = deferred<number>();
  let calls = 0;
  const published: number[] = [];
  const coordinator = new CatalogCoordinator(
    () => (++calls === 1 ? first.promise : Promise.resolve(2)),
    (value) => published.push(value),
  );
  const pending = coordinator.refresh();
  await Promise.resolve();
  coordinator.reset();
  const recovery = coordinator.refresh();
  assert.equal(calls, 1);
  first.resolve(1);
  assert.equal(await recovery, 2);
  assert.equal(await pending, 2);
  assert.deepEqual(published, [2]);
});

test('publication may request a new refresh without losing the invalidation', async () => {
  let calls = 0;
  let followup: Promise<number> | undefined;
  const published: number[] = [];
  const coordinator = new CatalogCoordinator(
    () => Promise.resolve(++calls),
    (value) => {
      published.push(value);
      if (value === 1) followup = coordinator.refresh(true);
    },
  );
  await coordinator.refresh();
  assert.equal(await followup, 2);
  assert.deepEqual(published, [1, 2]);
});

test('unmounted owners reject late callbacks and React effect replay can reactivate', async () => {
  let calls = 0;
  const coordinator = new CatalogCoordinator(
    () => Promise.resolve(++calls),
    () => undefined,
  );
  coordinator.deactivate();
  await assert.rejects(coordinator.refresh(), CatalogReadRetiredError);
  assert.equal(calls, 0);
  coordinator.activate();
  assert.equal(await coordinator.refresh(), 1);
});

test('aggregate diagnostics contain no catalog data and cannot alter publication', async () => {
  const events: unknown[] = [];
  const coordinator = new CatalogCoordinator(
    () => Promise.resolve({ privateId: 'excluded' }),
    () => undefined,
  );
  coordinator.observe(() => {
    throw new Error('observer failure');
  });
  coordinator.observe(async () => {
    throw new Error('async observer failure');
  });
  const stop = coordinator.observe((event) => {
    events.push(event);
  });
  assert.deepEqual(await coordinator.refresh(), { privateId: 'excluded' });
  assert.equal(events.length, 1);
  assert.deepEqual(Object.keys(events[0] as object).sort(), [
    'milliseconds',
    'outcome',
  ]);
  assert.equal((events[0] as { outcome: string }).outcome, 'published');
  stop();
  await coordinator.refresh();
  assert.equal(events.length, 1);
});
