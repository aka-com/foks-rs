import assert from 'node:assert/strict';
import test from 'node:test';

import { FIXTURE } from '../src/fixture';
import type { Server } from '../src/model';
import {
  LeaseExpiryCoordinator,
  reconcileObservedLeaseExpiries,
  type LeaseExpiryClock,
} from '../src/scheduling/lease-expiry';

class Clock implements LeaseExpiryClock {
  seconds = 1_000;
  private sequence = 0;
  timers = new Map<number, { at: number; callback: () => void }>();
  now = (): number => this.seconds;
  later = (callback: () => void, delayMs: number): unknown => {
    const id = ++this.sequence;
    this.timers.set(id, { at: this.seconds * 1_000 + delayMs, callback });
    return id;
  };
  cancel = (timer: unknown): void => {
    if (typeof timer === 'number') this.timers.delete(timer);
  };
  advance(seconds: number): void {
    this.seconds += seconds;
    for (;;) {
      const due = [...this.timers.entries()]
        .filter(([, timer]) => timer.at <= this.seconds * 1_000)
        .sort((left, right) => left[1].at - right[1].at)[0];
      if (!due) return;
      this.timers.delete(due[0]);
      due[1].callback();
    }
  }
}

function leased(id: string, expiresAt: number): Server {
  const base = FIXTURE.servers[0];
  assert.ok(base);
  return {
    ...base,
    id,
    name: `${id}.example.test`,
    compatibility: { status: 'required', expiresAt },
  };
}

test('expiry coordinator closes access at the exact earliest boundary and schedules later leases', () => {
  const clock = new Clock();
  const changes: { profiles: string[]; expired: string[] }[] = [];
  const coordinator = new LeaseExpiryCoordinator(clock, (change) =>
    changes.push({
      profiles: change.observed.map((entry) => entry.profile),
      expired: change.newlyExpired.map((entry) => entry.profile),
    }),
  );
  coordinator.update([leased('one', 1_010), leased('two', 1_020)]);
  clock.advance(9);
  assert.deepEqual(changes, []);
  clock.advance(1);
  assert.deepEqual(changes.at(-1), { profiles: ['one'], expired: ['one'] });
  clock.advance(10);
  assert.deepEqual(changes.at(-1), {
    profiles: ['one', 'two'],
    expired: ['two'],
  });
  coordinator.dispose();
});

test('expiry timer retains sub-second precision at the signed boundary', () => {
  const clock = new Clock();
  clock.seconds = 1_009.9;
  let expired = false;
  const coordinator = new LeaseExpiryCoordinator(clock, (change) => {
    expired ||= change.newlyExpired.length > 0;
  });
  coordinator.update([leased('one', 1_010)]);
  const timer = [...clock.timers.values()][0];
  assert.ok(timer);
  assert.ok(Math.abs(timer.at - 1_010_000) < 0.001);
  clock.advance(0.099);
  assert.equal(expired, false);
  clock.advance(0.001);
  assert.equal(expired, true);
  coordinator.dispose();
});

test('foreground reconciliation handles sleep and forward clock jumps', () => {
  const clock = new Clock();
  const expired: string[][] = [];
  const coordinator = new LeaseExpiryCoordinator(clock, (change) =>
    expired.push(change.newlyExpired.map((entry) => entry.profile)),
  );
  coordinator.update([leased('one', 1_010)]);
  clock.seconds = 2_000;
  coordinator.foreground();
  assert.deepEqual(expired, [['one']]);
  coordinator.dispose();
});

test('same or older authenticated lease cannot reopen after a backwards clock change', () => {
  const server = leased('one', 1_000);
  const observed = reconcileObservedLeaseExpiries([server], [], 1_000);
  assert.deepEqual(observed, [{ profile: 'one', expiresAt: 1_000 }]);
  assert.deepEqual(
    reconcileObservedLeaseExpiries([server], observed, 900),
    observed,
  );
  assert.deepEqual(
    reconcileObservedLeaseExpiries([leased('one', 999)], observed, 900),
    observed,
  );
  assert.deepEqual(
    reconcileObservedLeaseExpiries([leased('one', 1_100)], observed, 900),
    [],
  );
});

test('not-required clears an observation while unknown retains it and removal drops it', () => {
  const base = leased('one', 1_000);
  const observed = [{ profile: 'one', expiresAt: 1_000 }];
  assert.deepEqual(
    reconcileObservedLeaseExpiries(
      [
        {
          ...base,
          compatibility: {
            status: 'requirement-unknown',
            error: {
              code: 'offline',
              message: 'varied text',
              fatal: false,
              retryable: true,
              ambiguous: false,
            },
          },
        },
      ],
      observed,
      900,
    ),
    observed,
  );
  assert.deepEqual(
    reconcileObservedLeaseExpiries(
      [{ ...base, compatibility: { status: 'not-required' } }],
      observed,
      900,
    ),
    [],
  );
  assert.deepEqual(reconcileObservedLeaseExpiries([], observed, 900), []);
});

test('dispose cancels the outstanding timer', () => {
  const clock = new Clock();
  const coordinator = new LeaseExpiryCoordinator(clock, () => undefined);
  coordinator.update([leased('one', 2_000)]);
  assert.equal(clock.timers.size, 1);
  coordinator.dispose();
  assert.equal(clock.timers.size, 0);
});

test('an older seeded observation cannot lower the fail-closed floor', () => {
  const clock = new Clock();
  clock.seconds = 1_600;
  const coordinator = new LeaseExpiryCoordinator(clock, () => undefined);
  coordinator.update([leased('one', 1_500)]);
  clock.seconds = 1_000;
  coordinator.update(
    [leased('one', 1_500)],
    [{ profile: 'one', expiresAt: 1_200 }],
  );
  assert.deepEqual(coordinator.snapshot(), [
    { profile: 'one', expiresAt: 1_500 },
  ]);
  coordinator.dispose();
});

test('a reentrant callback does not leave a stale second timer', () => {
  const clock = new Clock();
  const owner: { current?: LeaseExpiryCoordinator } = {};
  const coordinator = new LeaseExpiryCoordinator(clock, () =>
    owner.current?.dispose(),
  );
  owner.current = coordinator;
  coordinator.update([leased('one', 900)]);
  assert.equal(clock.timers.size, 0);
});
