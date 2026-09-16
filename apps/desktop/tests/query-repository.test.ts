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
  await query.load();
  old.resolve('old');
  await assert.rejects(pending, RetiredQueryError);
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
