import assert from 'node:assert/strict';
import test from 'node:test';
import { DesktopReconciliation } from '../src/desktop-reconciliation';
import { FIXTURE } from '../src/fixture';
import type { AgentSnapshot } from '../src/model';
import {
  ReconciliationScheduler,
  type ReconciliationClock,
  type ReconciliationJob,
} from '../src/scheduling/reconciliation';

class Clock implements ReconciliationClock {
  time = 0;
  next = 0;
  timers = new Map<number, { at: number; run: () => void }>();
  now = () => this.time;
  random = () => 0;
  later = (run: () => void, delay: number) => {
    const id = ++this.next;
    this.timers.set(id, { at: this.time + delay, run });
    return id;
  };
  cancel = (id: unknown) => {
    this.timers.delete(id as number);
  };
  async advance(milliseconds: number) {
    const end = this.time + milliseconds;
    for (;;) {
      const next = [...this.timers].sort((a, b) => a[1].at - b[1].at)[0];
      if (!next || next[1].at > end) break;
      this.time = next[1].at;
      this.timers.delete(next[0]);
      next[1].run();
      await flush();
    }
    this.time = end;
    await flush();
  }
}
const flush = async () => {
  for (let i = 0; i < 30; i++) await Promise.resolve();
};
function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((yes) => {
    resolve = yes;
  });
  return { promise, resolve };
}
function job(
  key: string,
  run: ReconciliationJob['run'],
  scope: string | null = key,
): ReconciliationJob {
  return { key, scope, kind: 'catalog', interval: 30_000, run };
}

test('routine triggers coalesce without retiring a slow result; invalidation adds one trailing read', async () => {
  const clock = new Clock(),
    gate = deferred();
  const scheduler = new ReconciliationScheduler(clock);
  let calls = 0,
    published = 0;
  scheduler.update([
    job('p', async (context) => {
      calls++;
      if (calls === 1) await gate.promise;
      if (context.isCurrent()) published++;
    }),
  ]);
  scheduler.setEnabled(true);
  scheduler.request('p', 'foreground');
  await clock.advance(0);
  for (let i = 0; i < 10; i++) scheduler.request('p', 'periodic');
  await clock.advance(90_000);
  assert.equal(calls, 1);
  scheduler.request('p', 'mutation', true);
  scheduler.request('p', 'mutation', true);
  gate.resolve();
  await flush();
  await clock.advance(1_000);
  assert.equal(calls, 2);
  assert.equal(published, 2);
  scheduler.dispose();
});

test('failing profile backs off independently and emits identity-free diagnostics', async () => {
  const clock = new Clock(),
    events: unknown[] = [];
  const scheduler = new ReconciliationScheduler(clock);
  scheduler.observe((event) => {
    events.push(event);
  });
  let failed = 0,
    healthy = 0;
  scheduler.update([
    job('private-profile', async () => {
      failed++;
      throw new Error('private-value');
    }),
    job('healthy-profile', async () => {
      healthy++;
    }),
  ]);
  scheduler.setEnabled(true);
  scheduler.requestAll('foreground');
  await clock.advance(0);
  assert.equal(failed, 1);
  assert.equal(healthy, 1);
  await clock.advance(999);
  assert.equal(failed, 1);
  await clock.advance(1);
  assert.equal(failed, 2);
  await clock.advance(2_000);
  assert.equal(failed, 3);
  assert.equal(healthy, 1);
  assert.doesNotMatch(JSON.stringify(events), /private|healthy-profile/);
  assert.equal(scheduler.snapshot('private-profile')?.lastSuccessAt, undefined);
  assert.equal(scheduler.snapshot('healthy-profile')?.lastSuccessAt, 0);
  scheduler.dispose();
});

test('pause retires publication but does not release capacity before actual settlement', async () => {
  const clock = new Clock(),
    gate = deferred();
  const scheduler = new ReconciliationScheduler(clock, 1);
  let published = 0,
    started = 0;
  const work = job('p', async (context) => {
    started++;
    await gate.promise;
    if (context.isCurrent()) published++;
  });
  scheduler.update([work]);
  scheduler.setEnabled(true);
  scheduler.request('p', 'foreground');
  await clock.advance(0);
  scheduler.setEnabled(false);
  scheduler.update([
    job('q', async () => {
      started++;
    }),
  ]);
  scheduler.setEnabled(true);
  scheduler.requestAll('foreground');
  await clock.advance(0);
  assert.equal(started, 1);
  gate.resolve();
  await flush();
  await clock.advance(0);
  assert.equal(published, 0);
  assert.equal(started, 2);
  scheduler.dispose();
  assert.equal(clock.timers.size, 0);
});

test('root jobs exclude profile jobs and hidden windows catch up only once', async () => {
  const clock = new Clock(),
    gate = deferred();
  const scheduler = new ReconciliationScheduler(clock);
  const order: string[] = [];
  scheduler.update([
    job(
      'root',
      async () => {
        order.push('root');
        await gate.promise;
      },
      null,
    ),
    job('p', async () => {
      order.push('p');
    }),
  ]);
  scheduler.setEnabled(true);
  scheduler.requestAll('foreground');
  await clock.advance(0);
  assert.deepEqual(order, ['root']);
  scheduler.setVisible(false);
  gate.resolve();
  await flush();
  await clock.advance(300_000);
  assert.deepEqual(order, ['root']);
  scheduler.setVisible(true);
  await clock.advance(0);
  assert.equal(order.filter((entry) => entry === 'p').length, 1);
  scheduler.dispose();
});

test('ineligible operations pause without requiring healthy catalog data for other repair jobs', async () => {
  const clock = new Clock();
  const scheduler = new ReconciliationScheduler(clock);
  let metadata = 0,
    vault = 0;
  scheduler.update([
    {
      ...job(
        'vault',
        async () => {
          vault++;
        },
        'p',
      ),
      eligible: () => false,
    },
    job(
      'metadata',
      async () => {
        metadata++;
      },
      'p',
    ),
  ]);
  scheduler.setEnabled(true);
  scheduler.requestAll('foreground');
  await clock.advance(0);
  assert.equal(vault, 0);
  assert.equal(metadata, 1);
  scheduler.dispose();
});

test('fatal integrity errors require explicit retry while cancellation is not reported as failure', async () => {
  const clock = new Clock(),
    scheduler = new ReconciliationScheduler(clock);
  let calls = 0;
  scheduler.update([
    job('p', async () => {
      calls++;
      throw { code: 'response-binding', fatal: true, retryable: false };
    }),
  ]);
  scheduler.setEnabled(true);
  scheduler.requestAll('foreground');
  await clock.advance(0);
  scheduler.requestAll('network');
  await clock.advance(300_000);
  assert.equal(calls, 1);
  scheduler.request('p', 'manual');
  await clock.advance(0);
  assert.equal(calls, 2);
  scheduler.update([
    job('q', async () => {
      throw { code: 'cancelled' };
    }),
  ]);
  scheduler.request('q', 'foreground');
  await clock.advance(0);
  assert.equal(scheduler.snapshot('q')?.error, undefined);
  assert.equal(scheduler.snapshot('q')?.lastSuccessAt, undefined);
  scheduler.dispose();
});

test('desktop scheduling discovers already-bound accounts repeatedly without running while disabled', async () => {
  const clock = new Clock();
  const snapshot: AgentSnapshot = {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server) => ({
      ...server,
      trust: { status: 'verified' },
      compatibility: { status: 'not-required' },
      passiveStatus: { status: 'available', source: 'signed-server-status' },
      restrictions: [],
    })),
    profileInventory: FIXTURE.servers.map((server) => ({
      profile: server.id,
      accounts: 'complete',
      teams: 'complete',
    })),
  };
  const bound = snapshot.accounts.find((account) =>
    snapshot.stores.some(
      (store) =>
        store.kind === 'team' &&
        store.account === account.alias &&
        store.server === account.server,
    ),
  );
  assert.ok(bound);
  const accounts: string[] = [];
  const service = new DesktopReconciliation(
    {
      snapshot: () => snapshot,
      nowSeconds: () => clock.now() / 1_000,
      profile: async () => {},
      registry: async () => {},
      metadata: async () => {},
      discovery: async (account) => {
        accounts.push(account.store);
      },
    },
    clock,
  );
  service.update(snapshot);
  await clock.advance(10_000);
  assert.equal(accounts.length, 0);
  service.scheduler.setEnabled(true);
  await clock.advance(0);
  assert.ok(accounts.includes(bound.store));
  const first = accounts.filter((store) => store === bound.store).length;
  await clock.advance(300_000);
  assert.equal(
    accounts.filter((store) => store === bound.store).length,
    first + 1,
  );
  service.dispose();
});
