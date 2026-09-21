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

test('failing profile backs off independently and emits error-free diagnostics', async () => {
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
  // The job's key and scope are reported; the error it threw is not.
  assert.doesNotMatch(JSON.stringify(events), /private-value/);
  assert.ok(
    events.some(
      (event) =>
        (event as { key: string; scope: string }).key === 'private-profile' &&
        (event as { key: string; scope: string }).scope === 'private-profile',
    ),
  );
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
  // The snapshot says the job is parked, so what is shown about it can say
  // the same rather than re-deriving it from the error.
  assert.equal(scheduler.snapshot('p')?.paused, true);
  scheduler.requestAll('network');
  await clock.advance(300_000);
  assert.equal(calls, 1);
  scheduler.request('p', 'manual');
  await clock.advance(0);
  assert.equal(calls, 2);
  assert.equal(scheduler.snapshot('p')?.paused, true);
  scheduler.reconciled('p');
  assert.equal(scheduler.snapshot('p')?.paused, false);
  // A metadata job's fatal failure does not park it, and its snapshot agrees.
  scheduler.update([
    {
      ...job('m', async () => {
        throw { code: 'response-binding', fatal: true, retryable: false };
      }),
      kind: 'metadata',
    },
  ]);
  scheduler.request('m', 'foreground');
  await clock.advance(0);
  assert.ok(scheduler.snapshot('m')?.error);
  assert.equal(scheduler.snapshot('m')?.paused, false);
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

test('desktop scheduling discovers an account that is bound to no team', async () => {
  const clock = new Clock();
  const base: AgentSnapshot = {
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
  // Launch no longer runs its own pass over the unbound accounts, so the
  // periodic job has to reach them.
  const snapshot: AgentSnapshot = {
    ...base,
    stores: base.stores.filter((store) => store.kind !== 'team'),
    storeInventory: base.storeInventory.filter((entry) =>
      base.stores.some(
        (store) => store.id === entry.store && store.kind !== 'team',
      ),
    ),
  };
  const unbound = snapshot.accounts.find((account) =>
    snapshot.stores.some(
      (store) =>
        store.kind === 'account' &&
        store.id === account.store &&
        store.server === account.server &&
        store.account === account.alias,
    ),
  );
  assert.ok(unbound);
  assert.equal(
    snapshot.stores.some((store) => store.kind === 'team'),
    false,
  );
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
  service.scheduler.setEnabled(true);
  await clock.advance(10_000);
  assert.ok(accounts.includes(unbound.store));
  service.dispose();
});

test('a snapshot names the next attempt while the job is idle, and none while it runs or is parked', async () => {
  const clock = new Clock(),
    gate = deferred();
  let failing = false;
  const scheduler = new ReconciliationScheduler(clock);
  scheduler.update([
    {
      ...job('p', async () => {
        if (failing)
          throw {
            code: 'response-binding',
            message: 'The reply did not match.',
            fatal: true,
            retryable: false,
            ambiguous: false,
          };
        await gate.promise;
      }),
      initialDelay: 5_000,
    },
  ]);
  scheduler.setEnabled(true);
  assert.equal(scheduler.snapshot('p')?.nextAttemptAt, 5_000);
  await clock.advance(5_000);
  assert.equal(scheduler.snapshot('p')?.refreshing, true);
  assert.equal(scheduler.snapshot('p')?.nextAttemptAt, undefined);
  gate.resolve();
  await flush();
  assert.equal(scheduler.snapshot('p')?.refreshing, false);
  assert.equal(scheduler.snapshot('p')?.nextAttemptAt, 5_000 + 30_000);
  assert.equal(
    scheduler.observations().find((entry) => entry.key === 'p')?.snapshot
      .nextAttemptAt,
    5_000 + 30_000,
  );
  // A fatal failure parks the job: it is paused and has no next attempt.
  failing = true;
  scheduler.request('p', 'manual');
  await clock.advance(1_000);
  assert.equal(scheduler.snapshot('p')?.paused, true);
  assert.equal(scheduler.snapshot('p')?.nextAttemptAt, undefined);
});

test('run waits for the first execution started after the request', async () => {
  const clock = new Clock(),
    gates = [deferred(), deferred()];
  const scheduler = new ReconciliationScheduler(clock);
  let calls = 0;
  scheduler.update([
    job('p', async () => {
      await gates[calls++].promise;
    }),
  ]);
  scheduler.setEnabled(true);
  scheduler.request('p', 'foreground');
  await clock.advance(0);
  assert.equal(calls, 1);
  let settled = false;
  const awaited = scheduler.run('p', 'mutation');
  assert.ok(awaited);
  void awaited.then(() => {
    settled = true;
  });
  gates[0].resolve();
  await flush();
  await clock.advance(0);
  // The first run started before the request, so the trailing run answers it.
  assert.equal(calls, 2);
  assert.equal(settled, false);
  gates[1].resolve();
  await flush();
  assert.equal(settled, true);
  scheduler.dispose();
});

test('run propagates job failures and returns null for unavailable jobs', async () => {
  const clock = new Clock();
  const scheduler = new ReconciliationScheduler(clock);
  let eligible = true;
  scheduler.update([
    {
      ...job('p', async () => {
        throw { code: 'io', message: 'offline', retryable: true };
      }),
      eligible: () => eligible,
    },
  ]);
  // A scheduler that is not running has no run to wait for.
  assert.equal(scheduler.run('p', 'mutation'), null);
  scheduler.setEnabled(true);
  assert.equal(scheduler.run('missing', 'mutation'), null);
  const failed = scheduler.run('p', 'mutation');
  assert.ok(failed);
  const rejection = assert.rejects(failed, { code: 'io' });
  await clock.advance(0);
  await rejection;
  // An ineligible job is never started, so it is declined outright.
  eligible = false;
  assert.equal(scheduler.run('p', 'mutation'), null);
  scheduler.dispose();
});

test('a job that becomes ineligible rejects its waiters', async () => {
  const clock = new Clock();
  const scheduler = new ReconciliationScheduler(clock);
  let eligible = true,
    ran = 0;
  scheduler.update([
    {
      ...job('p', async () => {
        ran++;
      }),
      eligible: () => eligible,
    },
  ]);
  scheduler.setEnabled(true);
  const awaited = scheduler.run('p', 'mutation');
  assert.ok(awaited);
  const rejection = assert.rejects(awaited, { code: 'catalog-read-retired' });
  eligible = false;
  // Any later scheduling decision is where the job is found unrunnable.
  scheduler.setVisible(true);
  await rejection;
  await clock.advance(60_000);
  assert.equal(ran, 0);
  scheduler.dispose();
});

test('stopping the scheduler rejects waiters for idle jobs', async () => {
  const clock = new Clock();
  for (const stop of ['disable', 'hide', 'remove', 'dispose'] as const) {
    const scheduler = new ReconciliationScheduler(clock);
    scheduler.update([job('p', async () => {})]);
    scheduler.setEnabled(true);
    const awaited = scheduler.run('p', 'mutation');
    assert.ok(awaited);
    const rejection = assert.rejects(awaited, {
      code: 'catalog-read-retired',
    });
    if (stop === 'disable') scheduler.setEnabled(false);
    if (stop === 'hide') scheduler.setVisible(false);
    if (stop === 'remove') scheduler.update([job('q', async () => {})]);
    if (stop === 'dispose') scheduler.dispose();
    await rejection;
    scheduler.dispose();
  }
});

test('stopping during an active job rejects waiters for its follow-up run', async () => {
  for (const stop of ['disable', 'hide'] as const) {
    const clock = new Clock(),
      gate = deferred();
    const scheduler = new ReconciliationScheduler(clock);
    let calls = 0;
    scheduler.update([
      job('p', async () => {
        if (++calls === 1) await gate.promise;
      }),
    ]);
    scheduler.setEnabled(true);
    scheduler.request('p', 'foreground');
    await clock.advance(0);
    assert.equal(calls, 1);
    // The request lands on a run that started before it, so a trailing run
    // owes the caller an answer — and the scheduler stops before it starts.
    const awaited = scheduler.run('p', 'mutation');
    assert.ok(awaited);
    const rejection = assert.rejects(awaited, {
      code: 'catalog-read-retired',
    });
    if (stop === 'disable') scheduler.setEnabled(false);
    else scheduler.setVisible(false);
    gate.resolve();
    await flush();
    await clock.advance(60_000);
    await rejection;
    assert.equal(calls, 1);
    // The trailing run is still owed and runs once the scheduler is running.
    if (stop === 'disable') scheduler.setEnabled(true);
    else scheduler.setVisible(true);
    await clock.advance(0);
    assert.equal(calls, 2);
    scheduler.dispose();
  }
});

test('a snapshot keeps how long the last run took, on success and on failure', async () => {
  const clock = new Clock(),
    gates = [deferred(), deferred()];
  const scheduler = new ReconciliationScheduler(clock);
  let calls = 0;
  scheduler.update([
    job('p', async () => {
      const run = calls++;
      await gates[run].promise;
      if (run === 1)
        throw Object.assign(new Error('unreachable'), { code: 'io' });
    }),
  ]);
  scheduler.setEnabled(true);
  scheduler.request('p', 'foreground');
  await clock.advance(0);
  await clock.advance(250);
  gates[0].resolve();
  await flush();
  // The duration is on the job's snapshot, so the row can state it whether
  // or not the observations are still held anywhere.
  assert.equal(scheduler.snapshot('p')?.lastMilliseconds, 250);
  // A request is held to a second past the last attempt, so the second run
  // starts at 1000 ms and its own duration is measured from there.
  scheduler.request('p', 'manual');
  await clock.advance(750);
  await clock.advance(80);
  gates[1].resolve();
  await flush();
  assert.ok(scheduler.snapshot('p')?.error);
  // A run that failed took as long as it took; the row says so beside the
  // failure rather than leaving the previous run's number standing.
  assert.equal(scheduler.snapshot('p')?.lastMilliseconds, 80);
  scheduler.dispose();
});
