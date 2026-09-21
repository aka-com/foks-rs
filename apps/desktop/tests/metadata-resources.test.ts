import assert from 'node:assert/strict';
import test, { mock } from 'node:test';
import type { AppInfo, Bridge } from '../src/bridge';
import { enqueueProfileWork } from '../src/bridge';
import type { TeamStore } from '../src/model';
import {
  MetadataRepository,
  RetiredQueryError,
} from '../src/metadata-repository';
import { QueryRepository } from '../src/query-repository';
import {
  MetadataRepositoryContext,
  QueryRepositoryContext,
  useMetadataRepository,
  useQueryRepository,
} from '../src/query-hooks';
import { appInfoKey, appInfoQuery } from '../src/resources/application';
import {
  invitationRecoveryQuery,
  pendingOperationsQuery,
  reportableTeamRequestError,
  teamRequestCountQuery,
  TEAM_REQUEST_FRESHNESS,
} from '../src/operation-queries';

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

const tick = () => new Promise<void>((done) => setImmediate(done));

test('legacy metadata names are aliases, not separate repositories or contexts', () => {
  assert.equal(QueryRepository, MetadataRepository);
  assert.equal(QueryRepositoryContext, MetadataRepositoryContext);
  assert.equal(useQueryRepository, useMetadataRepository);
  assert.ok(new QueryRepository() instanceof MetadataRepository);
});

test('app info coalesces, publishes only display metadata, and remains reusable after unsubscribe', async () => {
  let now = 0;
  let calls = 0;
  const repository = new MetadataRepository(() => now);
  const reply = deferred<AppInfo>();
  const bridge = {
    appInfo: () => {
      calls++;
      return reply.promise;
    },
  };
  const query = appInfoQuery(repository, bridge);
  const unsubscribe = query.subscribe(() => {});
  const pending = query.load();
  assert.equal(appInfoQuery(repository, bridge), query);
  assert.equal(appInfoQuery(repository, bridge).load(), pending);
  await tick();
  assert.equal(calls, 1);
  const source = {
    version: '1',
    agentSocket: '/agent.sock',
    managedProfile: 'local',
    computerName: 'Desktop',
    token: 'do-not-cache',
    pin: 'do-not-cache',
    phrase: 'do-not-cache',
    value: 'do-not-cache',
    pid: 123,
  };
  reply.resolve(source);
  const data = await pending;
  assert.deepEqual(data, {
    version: '1',
    agentSocket: '/agent.sock',
    managedProfile: 'local',
    computerName: 'Desktop',
  });
  source.version = 'changed';
  assert.equal(data.version, '1');
  assert.ok(Object.isFrozen(data));
  unsubscribe();
  await query.load();
  assert.equal(calls, 1);
  now = 60_000;
  await repository.reconcileSubscribed();
  assert.equal(calls, 1);
  const stop = query.subscribe(() => {});
  await repository.reconcileSubscribed();
  assert.equal(calls, 2);
  stop();
});

test('app info invalidation queues one trailing read and retirement drops metadata', async () => {
  const repository = new MetadataRepository();
  const first = deferred<AppInfo>();
  let calls = 0;
  const query = appInfoQuery(repository, {
    appInfo: () =>
      ++calls === 1
        ? first.promise
        : Promise.resolve({ version: 'new', agentSocket: '/new.sock' }),
  });
  const pending = query.load();
  await tick();
  repository.invalidate(appInfoKey);
  const trailing = query.load();
  repository.invalidate(appInfoKey);
  assert.equal(query.load(), trailing);
  first.resolve({ version: 'old', agentSocket: '/old.sock' });
  await assert.rejects(pending, RetiredQueryError);
  assert.equal((await trailing).version, 'new');
  assert.equal(calls, 2);
  repository.retire();
  assert.equal(query.getSnapshot().data, undefined);
  await assert.rejects(query.load(), RetiredQueryError);
});

test('pending-operation resources retain only public recovery metadata and coalesce per profile', async () => {
  const repository = new MetadataRepository();
  let calls = 0;
  const source = [
    {
      kind: 'account-recovery' as const,
      alias: 'account',
      target: 'owner',
      phrase: 'private-phrase',
      token: 'private-token',
      pin: 'private-pin',
      state: { running: true },
    },
  ];
  const bridge = {
    listPendingOperations: async () => {
      calls++;
      return source;
    },
  } as unknown as Bridge;
  const query = pendingOperationsQuery(repository, bridge, 'profile');
  assert.equal(pendingOperationsQuery(repository, bridge, 'profile'), query);
  const pending = query.load();
  assert.equal(query.load(), pending);
  assert.deepEqual(await pending, [
    {
      kind: 'account-recovery',
      alias: 'account',
      target: 'owner',
    },
  ]);
  assert.equal(calls, 1);
  assert.notEqual(pendingOperationsQuery(repository, bridge, 'other'), query);
});

test('invitation resources publish counts without tokens, phrases, PINs, or reply records', async () => {
  const repository = new MetadataRepository();
  const events: unknown[] = [];
  repository.observe((event) => {
    events.push(event);
  });
  let calls = 0;
  const bridge: Pick<Bridge, 'invitation'> = {
    invitation: async (_profile, _account, action, pin) => {
      calls++;
      assert.equal(pin, null);
      return action.action === 'list'
        ? [
            { team_id: 'team', state: 'pending', invite: 'private-token' },
            { team_id: 'other', state: 'pending', invite: 'private-token' },
            { team_id: 'team', state: 'cancelled', invite: 'private-token' },
          ]
        : {
            rows: [
              {
                request_id: 'request',
                state: 'pending',
                invite: 'private-token',
              },
              {
                request_id: 'done',
                state: 'complete',
                invite: 'private-token',
              },
            ],
          };
    },
  };
  const store: TeamStore = {
    id: 'store',
    kind: 'team',
    name: 'Team',
    server: 'profile',
    account: 'account',
    alias: 'team',
    team_id_hex: 'team',
    active: true,
    team_kind: 'named',
  };
  const query = invitationRecoveryQuery(repository, bridge as Bridge, store);
  const pending = query.load();
  assert.equal(query.load(), pending);
  assert.equal(await pending, 2);
  assert.equal(query.getSnapshot().data, 2);
  assert.equal(calls, 2);
  assert.equal(JSON.stringify(events).includes('private-token'), false);
  for (const event of events) {
    assert.ok(
      Object.keys(event as object).every(
        (key) => key === 'kind' || key === 'milliseconds',
      ),
    );
  }
});

test('team request count does not re-enter a bridge that owns its own profile admission', async () => {
  // The native bridge queues `invitation` on the profile itself. A query that
  // queued the call again would hold the profile's slot while awaiting a
  // request that cannot start until the slot is released, and the inner
  // request would only settle at the admission deadline. Timers are mocked
  // so a regression fails here instead of after a real minute.
  mock.timers.enable({ apis: ['setTimeout'] });
  let dispatched = 0;
  const bridge = {} as Bridge;
  Object.assign(bridge, {
    invitation: (profile: string) =>
      enqueueProfileWork(bridge, profile, async () => {
        dispatched++;
        return { rows: [{ request_id: 'a' }, { request_id: 'b' }] };
      }),
  });
  const store: TeamStore = {
    id: 'store',
    kind: 'team',
    name: 'Team',
    server: 'profile',
    account: 'account',
    alias: 'team',
    team_id_hex: 'team',
    active: true,
    team_kind: 'named',
  };
  const repository = new MetadataRepository(() => 0);
  const pending = teamRequestCountQuery(repository, bridge, store).load();
  let settled: 'pending' | 'resolved' | 'rejected' = 'pending';
  void pending.then(
    () => (settled = 'resolved'),
    () => (settled = 'rejected'),
  );
  try {
    for (let round = 0; round < 8; round++) await tick();
    assert.equal(dispatched, 1);
    assert.equal(settled, 'resolved');
    assert.equal(await pending, 2);
  } finally {
    // Unwind any admission timer a regression left behind before restoring.
    mock.timers.tick(60_000);
    mock.timers.reset();
  }
});

test('team request counts are quiet about servers that are away and stay current for minutes', async () => {
  const cases = [
    ['io', false],
    ['profile-busy', false],
    ['deadline-exceeded', false],
    ['server-unavailable', false],
    ['catalog-required', false],
    ['agent-lost', true],
    ['unsafe-socket', true],
  ] as const;
  for (const [code, reported] of cases)
    assert.equal(
      reportableTeamRequestError({
        code,
        message: code,
        retryable: false,
        fatal: false,
        ambiguous: false,
      }),
      reported,
      code,
    );
  let now = 0;
  let calls = 0;
  const repository = new MetadataRepository(() => now);
  const bridge = {
    invitation: async () => {
      calls++;
      return { rows: [] };
    },
  } as unknown as Bridge;
  const store: TeamStore = {
    id: 'store',
    kind: 'team',
    name: 'Team',
    server: 'profile',
    account: 'account',
    alias: 'team',
    team_id_hex: 'team',
    active: true,
    team_kind: 'named',
  };
  const query = teamRequestCountQuery(repository, bridge, store);
  assert.equal(await query.load(), 0);
  now = TEAM_REQUEST_FRESHNESS - 1;
  await query.load();
  assert.equal(calls, 1);
  now = TEAM_REQUEST_FRESHNESS + 1;
  await query.load();
  assert.equal(calls, 2);
});
