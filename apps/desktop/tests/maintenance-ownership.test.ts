import assert from 'node:assert/strict';
import test from 'node:test';
import { MaintenanceOwnership } from '../src/app/maintenance-ownership';

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((yes) => {
    resolve = yes;
  });
  return { promise, resolve };
}

test('handoff retires bootstrap ingestion once without retiring shell ownership', () => {
  const ownership = new MaintenanceOwnership();
  const bootstrap = ownership.acquire();
  let bootStops = 0;
  bootstrap.install(() => bootStops++);
  assert.equal(
    bootstrap.handoff(() => true),
    true,
  );
  assert.equal(bootstrap.handedOff, true);
  assert.equal(bootstrap.isCurrent(), false);
  assert.equal(bootStops, 1);
  const shell = ownership.acquire();
  let shellStops = 0;
  shell.install(() => shellStops++);
  assert.equal(
    bootstrap.handoff(() => true),
    true,
  );
  bootstrap.retire();
  bootstrap.retire();
  assert.equal(bootStops, 1);
  assert.equal(shell.isCurrent(), true);
  assert.equal(shellStops, 0);
  shell.retire();
  shell.retire();
  assert.equal(shellStops, 1);
  assert.equal(
    bootstrap.handoff(() => true),
    false,
  );
});

test('a late bootstrap subscription is cleaned up after retirement', async () => {
  const ownership = new MaintenanceOwnership();
  const owner = ownership.acquire();
  const subscribed = deferred<() => void>();
  const registration = subscribed.promise.then((stop) => owner.install(stop));
  owner.retire();
  const replacement = ownership.acquire();
  let stops = 0;
  subscribed.resolve(() => stops++);
  await registration;
  assert.equal(stops, 1);
  assert.equal(owner.isCurrent(), false);
  assert.equal(replacement.isCurrent(), true);
  owner.retire();
  assert.equal(stops, 1);
});

test('a late subscription is cleaned up after handoff', async () => {
  const ownership = new MaintenanceOwnership();
  const owner = ownership.acquire();
  const subscribed = deferred<() => void>();
  const registration = subscribed.promise.then((stop) => owner.install(stop));
  assert.equal(
    owner.handoff(() => true),
    true,
  );
  let stops = 0;
  subscribed.resolve(() => stops++);
  await registration;
  owner.retire();
  assert.equal(stops, 1);
});

test('handoff checks publication currentness before and after releasing ingestion', () => {
  const ownership = new MaintenanceOwnership();
  const owner = ownership.acquire();
  let current = false;
  let stops = 0;
  owner.install(() => {
    stops++;
    current = false;
  });
  assert.equal(
    owner.handoff(() => current),
    false,
  );
  assert.equal(owner.isCurrent(), true);
  assert.equal(stops, 0);
  current = true;
  assert.equal(
    owner.handoff(() => current),
    false,
  );
  assert.equal(stops, 1);
  assert.equal(owner.handedOff, true);
  assert.equal(
    owner.handoff(() => current),
    false,
  );
  assert.equal(stops, 1);
});

test('replacing an active owner retires only its listener and stale callbacks are inert', () => {
  const ownership = new MaintenanceOwnership();
  const previous = ownership.acquire();
  const delivered: string[] = [];
  const oldCallback = () => {
    if (previous.isCurrent()) delivered.push('old');
  };
  let stops = 0;
  previous.install(() => stops++);
  const current = ownership.acquire();
  oldCallback();
  assert.equal(stops, 1);
  assert.equal(
    previous.handoff(() => true),
    false,
  );
  assert.equal(current.isCurrent(), true);
  assert.deepEqual(delivered, []);
});
