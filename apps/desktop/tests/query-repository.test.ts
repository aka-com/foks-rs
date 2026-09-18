import assert from 'node:assert/strict';
import test from 'node:test';
import { QueryRepository, RetiredQueryError } from '../src/query-repository';
import { ReadRecoveryCoordinator } from '../src/query-read-recovery';
import {
  invitationRecoveryQuery,
  pendingOperationsQuery,
} from '../src/operation-queries';
import type { Bridge } from '../src/bridge';
import type { TeamStore } from '../src/model';

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
const tick = () => new Promise<void>((done) => setImmediate(done));
const catalogMissing = {
  code: 'catalog-required',
  message: 'Missing catalog',
  retryable: true,
  fatal: false,
  ambiguous: false,
};

test('metadata singleflight publishes immutable detached data through stable snapshots', async () => {
  const repository = new QueryRepository();
  const reply = deferred<{ rows: { name: string }[] }>();
  let calls = 0;
  const query = repository.query(['devices', 'p', 'a'], () => {
    calls++;
    return reply.promise;
  });
  const states: unknown[] = [];
  const unsubscribe = query.subscribe(() => states.push(query.getSnapshot()));
  const a = query.load();
  const b = query.load();
  assert.equal(a, b);
  await tick();
  assert.equal(calls, 1);
  const source = { rows: [{ name: 'Original' }] };
  reply.resolve(source);
  const data = await a;
  source.rows[0].name = 'Changed outside the repository';
  assert.equal(data.rows[0].name, 'Original');
  assert.throws(() => {
    data.rows[0].name = 'Changed by a subscriber';
  }, TypeError);
  assert.equal(query.getSnapshot(), states[states.length - 1]);
  assert.equal(states.length, 2);
  await query.load();
  assert.equal(calls, 1);
  unsubscribe();
});

test('targeted invalidation preserves stale data and failures do not erase accepted values', async () => {
  let now = 0;
  const repository = new QueryRepository(() => now);
  let value = 'first';
  let failure = false;
  let profileReads = 0;
  const account = repository.query(['account-devices', 'p', 'a'], async () => {
    if (failure) throw new Error('offline');
    return value;
  });
  const profile = repository.query(['profile-enrollments', 'p'], async () => {
    profileReads++;
    return [];
  });
  await Promise.all([account.load(), profile.load()]);
  repository.invalidate(['account-devices', 'p']);
  assert.equal(account.getSnapshot().data, 'first');
  value = 'second';
  await Promise.all([account.load(), profile.load()]);
  assert.equal(profileReads, 1);
  now = 60_001;
  failure = true;
  await assert.rejects(account.load(), /offline/);
  assert.equal(account.getSnapshot().data, 'second');
  const error = account.getSnapshot().error;
  assert.equal(account.claimError(error), true);
  assert.equal(account.claimError(error), false);
});

test('access reset drops old replies; a retired scope cannot restart reads', async () => {
  const repository = new QueryRepository();
  const old = deferred<string>();
  let calls = 0;
  const query = repository.query(['metadata', 'p'], () =>
    ++calls === 1 ? old.promise : Promise.resolve('new'),
  );
  const pending = query.load();
  await tick();
  repository.clear();
  const fresh = query.load();
  old.resolve('old');
  await assert.rejects(pending, RetiredQueryError);
  assert.equal(await fresh, 'new');
  assert.equal(query.getSnapshot().data, 'new');
  repository.retire();
  assert.equal(query.getSnapshot().data, undefined);
  await assert.rejects(query.load(), RetiredQueryError);
  assert.equal(calls, 2);
});

test('an invalidated in-flight read cannot replace a newer resource generation', async () => {
  const repository = new QueryRepository();
  const old = deferred<string>();
  let calls = 0;
  const query = repository.query(['metadata', 'p'], () =>
    ++calls === 1 ? old.promise : Promise.resolve('new'),
  );
  const pending = query.load();
  await tick();
  repository.invalidate(['metadata']);
  const fresh = query.load();
  repository.invalidate(['metadata']);
  assert.equal(query.load(), fresh);
  await tick();
  assert.equal(calls, 1);
  assert.equal(query.getSnapshot().fetching, true);
  old.resolve('old');
  await assert.rejects(pending, RetiredQueryError);
  assert.equal(await fresh, 'new');
  assert.equal(calls, 2);
  assert.equal(query.getSnapshot().data, 'new');
});

test('catalog repair is shared by concurrent and delayed failures from the same read generation', async () => {
  const repository = new QueryRepository();
  const recovery = new ReadRecoveryCoordinator();
  const secondRead = deferred<string>();
  let firstCalls = 0;
  let secondCalls = 0;
  let refreshes = 0;
  const first = repository.query(['first'], async () => {
    if (++firstCalls === 1) throw catalogMissing;
    return 'first';
  });
  const second = repository.query(['second'], () =>
    ++secondCalls === 1 ? secondRead.promise : Promise.resolve('second'),
  );
  const options = recovery.options({
    refresh: async () => {
      refreshes++;
    },
  });
  const a = first.load(options);
  const b = second.load(options);
  assert.equal(await a, 'first');
  secondRead.reject(catalogMissing);
  assert.equal(await b, 'second');
  assert.equal(refreshes, 1);
  assert.equal(firstCalls, 2);
  assert.equal(secondCalls, 2);
});

test('read repair happens at most once and never replays ambiguous operations', async () => {
  const repository = new QueryRepository();
  const recovery = new ReadRecoveryCoordinator();
  let calls = 0;
  let refreshes = 0;
  const missing = repository.query(['missing'], async () => {
    calls++;
    throw catalogMissing;
  });
  const options = recovery.options({
    refresh: async () => {
      refreshes++;
    },
  });
  await assert.rejects(missing.load(options));
  assert.equal(calls, 2);
  assert.equal(refreshes, 1);
  const ambiguous = repository.query(['ambiguous'], async () => {
    throw { ...catalogMissing, ambiguous: true };
  });
  await assert.rejects(
    ambiguous.load(
      recovery.options({
        refresh: async () => {
          refreshes++;
        },
      }),
    ),
  );
  assert.equal(refreshes, 1);
});

test('retirement while catalog repair is outstanding prevents its read retry', async () => {
  const repository = new QueryRepository();
  const repaired = deferred<void>();
  let calls = 0;
  const query = repository.query(['read'], async () => {
    calls++;
    throw catalogMissing;
  });
  const pending = query.load(
    new ReadRecoveryCoordinator().options({ refresh: () => repaired.promise }),
  );
  await tick();
  repository.retire();
  repaired.resolve();
  await assert.rejects(pending, RetiredQueryError);
  assert.equal(calls, 1);
  assert.equal(query.getSnapshot().error, undefined);
});

test('invitation queries retain only recovery counts and pending metadata stays profile-scoped', async () => {
  const repository = new QueryRepository();
  const bridge = {
    invitation: async (
      _profile: string,
      _account: string,
      action: { action: string },
    ) =>
      action.action === 'list'
        ? [
            {
              team_id: 'team-id',
              state: 'prepared',
              invite: 'secret-invitation-token',
            },
          ]
        : [
            {
              request_id: 'request',
              state: 'pending',
              phrase: 'secret-phrase',
            },
          ],
    listPendingOperations: async (profile: string) => [
      { kind: 'team-member-edit', alias: profile },
    ],
  } as unknown as Bridge;
  const team = {
    server: 'p',
    account: 'a',
    alias: 'team',
    team_id_hex: 'team-id',
  } as TeamStore;
  const query = invitationRecoveryQuery(repository, bridge, team);
  assert.equal(await query.load(), 2);
  assert.equal(typeof query.getSnapshot().data, 'number');
  const [p, q] = await Promise.all([
    pendingOperationsQuery(repository, bridge, 'p').load(),
    pendingOperationsQuery(repository, bridge, 'q').load(),
  ]);
  assert.equal(p[0].alias, 'p');
  assert.equal(q[0].alias, 'q');
});

test('aggregate diagnostics omit identities and values and cannot affect read outcomes', async () => {
  const repository = new QueryRepository();
  const events: unknown[] = [];
  repository.observe((event) => {
    events.push(event);
  });
  repository.observe(() => {
    throw new Error('observer failure');
  });
  repository.observe(async () => {
    throw new Error('async observer failure');
  });
  const query = repository.query(
    ['private-profile', 'private-account'],
    async () => 'private-metadata',
  );
  const first = query.load();
  const coalesced = query.load();
  assert.equal(await first, 'private-metadata');
  await coalesced;
  await query.load();
  assert.deepEqual(
    events.map((event) => (event as { kind: string }).kind),
    ['load-start', 'coalesced', 'load-complete', 'cache-hit'],
  );
  assert.equal(JSON.stringify(events).includes('private-'), false);
  for (const event of events) {
    assert.ok(
      Object.keys(event as object).every(
        (key) => key === 'kind' || key === 'milliseconds',
      ),
    );
  }
});

test('reconciliation refreshes only due subscribed metadata and records attempt and success times', async () => {
  let now = 0;
  const repository = new QueryRepository(() => now);
  let calls = 0;
  const query = repository.query(['observed'], async () => ++calls, 100);
  let unseenCalls = 0;
  repository.query(['unobserved'], async () => ++unseenCalls, 100);
  const unsubscribe = query.subscribe(() => {});
  await repository.reconcileSubscribed();
  assert.equal(calls, 1);
  assert.equal(query.getSnapshot().lastAttemptAt, 0);
  assert.equal(query.getSnapshot().lastSuccessAt, 0);
  now = 99;
  await repository.reconcileSubscribed();
  assert.equal(calls, 1);
  now = 100;
  await repository.reconcileSubscribed();
  assert.equal(calls, 2);
  assert.equal(query.getSnapshot().data, 2);
  assert.equal(query.getSnapshot().lastAttemptAt, 100);
  assert.equal(query.getSnapshot().lastSuccessAt, 100);
  unsubscribe();
  now = 200;
  await repository.reconcileSubscribed();
  assert.equal(calls, 2);
  assert.equal(unseenCalls, 0);
});

test('reconciliation failures retain historical metadata with capped exponential backoff', async () => {
  let now = 0;
  let calls = 0;
  let fail = false;
  const repository = new QueryRepository(() => now);
  const query = repository.query(
    ['metadata'],
    async () => {
      calls++;
      if (fail) throw new Error('offline');
      return calls;
    },
    100,
  );
  query.subscribe(() => {});
  await query.load();
  fail = true;
  now = 100;
  for (const delay of [30_000, 60_000, 120_000, 240_000, 300_000, 300_000]) {
    await assert.rejects(repository.reconcileSubscribed(), /offline/);
    const attempts = calls;
    assert.equal(query.getSnapshot().data, 1);
    assert.equal(query.getSnapshot().lastSuccessAt, 0);
    assert.equal(query.getSnapshot().lastAttemptAt, now);
    assert.equal(query.getSnapshot().fetching, false);
    now += delay - 1;
    await repository.reconcileSubscribed();
    assert.equal(calls, attempts);
    now++;
  }
  fail = false;
  await repository.reconcileSubscribed();
  assert.equal(query.getSnapshot().lastSuccessAt, now);
  assert.equal(query.getSnapshot().error, undefined);
  fail = true;
  now += 100;
  await assert.rejects(repository.reconcileSubscribed(), /offline/);
  now += 30_000;
  await assert.rejects(repository.reconcileSubscribed(), /offline/);
  const attempts = calls;
  repository.invalidate(['metadata']);
  await assert.rejects(query.load(), /offline/);
  assert.equal(calls, attempts + 1);
});

test('reconciliation coalesces passes, caps concurrency and skips readers removed while waiting', async () => {
  const repository = new QueryRepository();
  const replies = Array.from({ length: 5 }, () => deferred<number>());
  const calls: number[] = [];
  let active = 0;
  let maximum = 0;
  const queries = replies.map((reply, index) =>
    repository.query([String(index)], async () => {
      calls.push(index);
      maximum = Math.max(maximum, ++active);
      try {
        return await reply.promise;
      } finally {
        active--;
      }
    }),
  );
  const unsubscribe = queries.map((query) => query.subscribe(() => {}));
  const pass = repository.reconcileSubscribed();
  assert.equal(repository.reconcileSubscribed(), pass);
  await tick();
  assert.deepEqual(calls, [0, 1]);
  unsubscribe[2]();
  replies[0].resolve(0);
  await tick();
  assert.deepEqual(calls, [0, 1, 3]);
  replies[1].resolve(1);
  await tick();
  assert.deepEqual(calls, [0, 1, 3, 4]);
  replies[3].resolve(3);
  replies[4].resolve(4);
  await pass;
  assert.equal(maximum, 2);
});

test('a failed reconciliation reader does not stop other readers or release the pass early', async () => {
  const repository = new QueryRepository();
  const slow = deferred<number>();
  let thirdCalls = 0;
  const failed = repository.query(['failed'], async () => {
    throw new Error('offline');
  });
  const pending = repository.query(['pending'], () => slow.promise);
  const third = repository.query(['third'], async () => ++thirdCalls);
  for (const query of [failed, pending, third]) query.subscribe(() => {});
  const pass = repository.reconcileSubscribed();
  const rejected = assert.rejects(pass, /offline/);
  await tick();
  assert.equal(thirdCalls, 1);
  assert.equal(repository.reconcileSubscribed(), pass);
  assert.equal(pending.getSnapshot().fetching, true);
  slow.resolve(1);
  await rejected;
  assert.equal(pending.getSnapshot().data, 1);
});

test('polling skips active invalidated work and does not request overlapping or extra reloads', async () => {
  const repository = new QueryRepository();
  const reply = deferred<string>();
  let calls = 0;
  const query = repository.query(['metadata'], () =>
    ++calls === 1 ? reply.promise : Promise.resolve('current'),
  );
  query.subscribe(() => {});
  const pending = query.load();
  await tick();
  repository.invalidate(['metadata']);
  for (let poll = 0; poll < 3; poll++) await repository.reconcileSubscribed();
  assert.equal(calls, 1);
  const trailing = query.load();
  assert.equal(query.load(), trailing);
  reply.resolve('retired');
  await assert.rejects(pending, RetiredQueryError);
  assert.equal(await trailing, 'current');
  await repository.reconcileSubscribed();
  assert.equal(calls, 2);
});

test('clear cancels queued reconciliation work and drops timestamps without losing outstanding reads', async () => {
  let now = 10;
  const repository = new QueryRepository(() => now);
  const replies = [deferred<number>(), deferred<number>()];
  const calls: number[] = [];
  const queries = [0, 1, 2].map((index) =>
    repository.query([String(index)], () => {
      calls.push(index);
      return replies[index]?.promise ?? Promise.resolve(index);
    }),
  );
  queries.forEach((query) => query.subscribe(() => {}));
  const pass = repository.reconcileSubscribed();
  await tick();
  repository.clear();
  assert.equal(queries[0].getSnapshot().lastAttemptAt, undefined);
  assert.equal(queries[0].getSnapshot().lastSuccessAt, undefined);
  assert.equal(queries[0].getSnapshot().fetching, true);
  assert.equal(repository.reconcileSubscribed(), pass);
  replies[0].resolve(0);
  replies[1].resolve(1);
  await pass;
  assert.deepEqual(calls, [0, 1]);
  assert.equal(queries[0].getSnapshot().data, undefined);
  assert.equal(queries[0].getSnapshot().fetching, false);
  now = 20;
  await repository.reconcileSubscribed();
  assert.deepEqual(calls, [0, 1, 0, 1, 2]);
  assert.equal(queries[0].getSnapshot().lastSuccessAt, 20);
});

test('retirement cancels a trailing reload and prevents later reconciliation', async () => {
  const repository = new QueryRepository();
  const reply = deferred<number>();
  let calls = 0;
  const query = repository.query(['metadata'], () => {
    calls++;
    return reply.promise;
  });
  query.subscribe(() => {});
  const pending = query.load();
  await tick();
  query.invalidate();
  const trailing = query.load();
  repository.retire();
  reply.resolve(1);
  await Promise.all([
    assert.rejects(pending, RetiredQueryError),
    assert.rejects(trailing, RetiredQueryError),
  ]);
  await repository.reconcileSubscribed();
  await assert.rejects(query.load(), RetiredQueryError);
  assert.equal(calls, 1);
  assert.equal(query.getSnapshot().data, undefined);
});

test('view recovery cannot bypass the session-wide repair admission policy', async () => {
  const repository = new QueryRepository();
  let allowed = true;
  let repairs = 0;
  const response = deferred<number>();
  repository.setReadRecovery(
    () => ({}),
    () => allowed,
  );
  const query = repository.query(['account'], () => response.promise);
  const pending = query.load({
    recover: async () => {
      repairs++;
      return true;
    },
  });
  await tick();
  allowed = false;
  response.reject(catalogMissing);
  await assert.rejects(pending);
  assert.equal(repairs, 0);
});
