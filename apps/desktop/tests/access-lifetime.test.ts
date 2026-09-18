import assert from 'node:assert/strict';
import test from 'node:test';
import { AccessLifetime } from '../src/app/access-lifetime';

test('captured work never becomes current again after a lock and a new capture', () => {
  const owner = new AccessLifetime();
  const old = owner.capture();
  owner.retire('lock');
  const next = owner.capture();
  assert.equal(old.isCurrent(), false);
  assert.equal(old.signal.aborted, true);
  assert.equal(next.isCurrent(), true);
  assert.notEqual(next.session, old.session);
  assert.ok(next.generation > old.generation);
});

test('profile expiry retires only that profile while global repair work remains current', () => {
  const owner = new AccessLifetime();
  const catalog = owner.capture();
  const first = owner.capture('first');
  const second = owner.capture('second');
  owner.retire('access-change', 'first');
  assert.equal(first.isCurrent(), false);
  assert.equal(second.isCurrent(), true);
  assert.equal(catalog.isCurrent(), true);
  assert.equal(first.signal.aborted, true);
  assert.equal(second.signal.aborted, false);
});

test('removing and reusing a profile name cannot revive an old ticket', () => {
  const owner = new AccessLifetime();
  const before = owner.capture('profile');
  owner.retainProfiles([]);
  const after = owner.capture('profile');
  assert.equal(before.isCurrent(), false);
  assert.equal(after.isCurrent(), true);
});

test('cleanup retires old work but Strict Mode setup can capture a fresh epoch', () => {
  const owner = new AccessLifetime();
  const first = owner.capture();
  let subscriptions = 0;
  const setup = () => {
    subscriptions++;
    const stop = owner.subscribe(() => {});
    return () => {
      stop();
      subscriptions--;
      owner.retire('access-change');
    };
  };
  const cleanup = setup();
  cleanup();
  const secondCleanup = setup();
  const second = owner.capture();
  assert.equal(subscriptions, 1);
  assert.equal(first.isCurrent(), false);
  assert.equal(second.isCurrent(), true);
  secondCleanup();
  assert.equal(subscriptions, 0);
  assert.equal(second.isCurrent(), false);
});

test('observer failures cannot prevent retirement and final disposal is exactly once', () => {
  const owner = new AccessLifetime();
  const first = owner.capture('first');
  let notifications = 0;
  owner.subscribe(() => {
    throw new Error('observer');
  });
  owner.subscribe(() => {
    notifications++;
  });
  owner.retire('maintenance');
  assert.equal(first.isCurrent(), false);
  assert.equal(notifications, 1);
  owner.dispose();
  owner.dispose();
  owner.retire('lock');
  assert.equal(notifications, 2);
  assert.equal(owner.capture().isCurrent(), false);
  assert.equal(owner.capture('new').signal.aborted, true);
});
