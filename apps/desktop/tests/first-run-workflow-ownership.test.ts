import assert from 'node:assert/strict';
import test from 'node:test';
import { AccessLifetime } from '../src/app/access-lifetime';
import {
  enqueueProfileWork,
  type Bridge,
  type GoProfileCandidate,
} from '../src/bridge';
import { FIXTURE } from '../src/fixture';
import {
  initialFirstRun,
  type FirstRunCheckpoint,
} from '../src/first-run-state';
import { runServerCheck } from '../src/screens/first-run/use-server-workflow';
import { runTeamDiscovery } from '../src/screens/first-run/use-team-discovery';
import {
  captureSetupWorkflow,
  releaseSetupWorkflow,
} from '../src/screens/first-run/workflow-ownership';

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

const profile = {
  profile: 'personal',
  acceptance: 'unchanged' as const,
  lookupName: 'localhost',
  canonicalName: 'localhost',
  hostId: `02${'2'.repeat(64)}`,
  chain: 1,
  epoch: 1,
};
const saved: FirstRunCheckpoint = {
  ...initialFirstRun('invited', 'waiting'),
  profile,
  serverAddress: 'localhost:4430',
  account: { alias: 'alice', username: 'alice', deviceName: 'Mac' },
};
const emptyGroups = { accountAlias: 'alice', groups: [] };

function ownership(initial: FirstRunCheckpoint = saved) {
  const lifetime = new AccessLifetime();
  let checkpoint = initial;
  let available = true;
  const capture = () =>
    captureSetupWorkflow(
      lifetime.capture(),
      checkpoint,
      () => checkpoint,
      () => available,
    );
  return {
    capture,
    replace: (next: FirstRunCheckpoint) => {
      checkpoint = next;
    },
    retire: () => lifetime.retire('access-change'),
    unmount: () => {
      available = false;
    },
  };
}

for (const labelFails of [false, true]) {
  test(`address editing during cosmetic label ${labelFails ? 'failure' : 'completion'} cannot publish a checked profile`, async () => {
    const owner = ownership({
      ...initialFirstRun('own', 'address'),
      serverAddress: 'localhost:4430',
    });
    const label = deferred<void>();
    const labelStarted = deferred<void>();
    let checks = 0;
    let labels = 0;
    const bridge = {
      checkAndAddProfile: async () => {
        checks++;
        return profile;
      },
      checkAndAddGoProfile: async () => {
        throw new Error('unexpected CLI check');
      },
      setServerLabel: async () => {
        labels++;
        labelStarted.resolve();
        await label.promise;
        return { profile: 'personal', label: 'localhost', changed: true };
      },
    } satisfies Parameters<typeof runServerCheck>[0]['bridge'];
    const pending = runServerCheck({
      bridge,
      address: 'localhost:4430',
      candidate: null,
      isCurrent: owner.capture(),
    });
    await labelStarted.promise;
    owner.replace({
      ...initialFirstRun('own', 'address'),
      serverAddress: 'other:4430',
    });
    owner.retire();
    const newer = owner.capture();
    if (labelFails) label.reject(new Error('cosmetic failure'));
    else label.resolve();
    assert.equal(await pending, null);
    assert.equal(newer(), true);
    assert.equal(checks, 1);
    assert.equal(labels, 1);
  });
}

test('cosmetic label failure is best-effort for the current address, without replay', async () => {
  let checks = 0;
  let labels = 0;
  const bridge = {
    checkAndAddProfile: async () => {
      checks++;
      return profile;
    },
    checkAndAddGoProfile: async () => {
      throw new Error('unexpected CLI check');
    },
    setServerLabel: async () => {
      labels++;
      throw new Error('label unavailable');
    },
  };
  assert.equal(
    await runServerCheck({
      bridge,
      address: 'localhost:4430',
      candidate: null,
      isCurrent: () => true,
    }),
    profile,
  );
  assert.equal(checks, 1);
  assert.equal(labels, 1);
});

test('server checking requires current ownership at dispatch and retains candidate host binding', async () => {
  let checks = 0;
  let labels = 0;
  const candidate: GoProfileCandidate = {
    candidateId: 'candidate',
    hostId: 'different-host',
    userId: 'user',
    deviceId: 'device',
    role: 'owner',
    storageKind: 'plaintext',
    hidden: false,
    provisional: false,
    pairable: true,
    copyable: true,
  };
  const bridge = {
    checkAndAddProfile: async () => {
      checks++;
      return profile;
    },
    checkAndAddGoProfile: async () => {
      checks++;
      return profile;
    },
    setServerLabel: async () => {
      labels++;
      throw new Error('unexpected label');
    },
  };
  assert.equal(
    await runServerCheck({
      bridge,
      address: 'localhost',
      candidate,
      isCurrent: () => false,
    }),
    null,
  );
  assert.equal(checks, 0);
  await assert.rejects(
    runServerCheck({
      bridge,
      address: 'localhost',
      candidate,
      isCurrent: () => true,
    }),
    /does not match/,
  );
  assert.equal(checks, 1);
  assert.equal(labels, 0);
});

for (const fails of [false, true]) {
  test(`old discovery ${fails ? 'failure' : 'completion'} after account change is ignored but refreshes the catalog`, async () => {
    const owner = ownership();
    const response = deferred<typeof emptyGroups>();
    const started = deferred<void>();
    let calls = 0;
    let refreshes = 0;
    const bridge = {
      discoverGroups: async (name: string, alias: string) => {
        assert.equal(name, profile.profile);
        assert.equal(alias, 'alice');
        calls++;
        started.resolve();
        return response.promise;
      },
    } as unknown as Bridge;
    const pending = runTeamDiscovery({
      bridge,
      saved,
      isCurrent: owner.capture(),
      onRefreshSnapshot: async (force) => {
        assert.equal(force, true);
        refreshes++;
        return FIXTURE;
      },
    });
    await started.promise;
    owner.replace({ ...saved, account: { ...saved.account!, alias: 'bob' } });
    const newer = owner.capture();
    if (fails) response.reject(new Error('old account failure'));
    else response.resolve(emptyGroups);
    assert.equal(await pending, null);
    assert.equal(newer(), true);
    assert.equal(calls, 1);
    assert.equal(refreshes, 1);
  });
}

for (const refreshFails of [false, true]) {
  test(`discovery refresh ${refreshFails ? 'failure' : 'completion'} cannot publish after account changes during refresh`, async () => {
    const owner = ownership();
    const refreshed = deferred<typeof FIXTURE>();
    const refreshing = deferred<void>();
    let calls = 0;
    const bridge = {
      discoverGroups: async () => {
        calls++;
        return emptyGroups;
      },
    } as unknown as Bridge;
    const pending = runTeamDiscovery({
      bridge,
      saved,
      isCurrent: owner.capture(),
      onRefreshSnapshot: async () => {
        refreshing.resolve();
        return refreshed.promise;
      },
    });
    await refreshing.promise;
    owner.replace({ ...saved, account: { ...saved.account!, alias: 'bob' } });
    if (refreshFails) refreshed.reject(new Error('old refresh failure'));
    else refreshed.resolve(FIXTURE);
    assert.equal(await pending, null);
    assert.equal(calls, 1);
  });
}

test('queued discovery checks captured ownership before mutation dispatch and never replays', async () => {
  const owner = ownership();
  const blocker = deferred<void>();
  const started = deferred<void>();
  let calls = 0;
  let refreshes = 0;
  const bridge = {
    discoverGroups: async () => {
      calls++;
      return emptyGroups;
    },
  } as unknown as Bridge;
  const blocking = enqueueProfileWork(bridge, profile.profile, async () => {
    started.resolve();
    await blocker.promise;
  });
  await started.promise;
  const pending = runTeamDiscovery({
    bridge,
    saved,
    isCurrent: owner.capture(),
    onRefreshSnapshot: async () => {
      refreshes++;
      return FIXTURE;
    },
  });
  owner.replace({ ...saved, account: { ...saved.account!, alias: 'bob' } });
  blocker.resolve();
  await blocking;
  assert.equal(await pending, null);
  assert.equal(calls, 0);
  assert.equal(refreshes, 0);
});

test('attempted discovery failure refreshes once and never retries the mutation', async () => {
  const failure = new Error('discovery failed');
  let calls = 0;
  let refreshes = 0;
  const bridge = {
    discoverGroups: async () => {
      calls++;
      throw failure;
    },
  } as unknown as Bridge;
  await assert.rejects(
    runTeamDiscovery({
      bridge,
      saved,
      isCurrent: () => true,
      onRefreshSnapshot: async (force) => {
        assert.equal(force, true);
        refreshes++;
        return FIXTURE;
      },
    }),
    (error) => error === failure,
  );
  assert.equal(calls, 1);
  assert.equal(refreshes, 1);
});

test('discovery keeps both account response bindings after refreshing', async () => {
  for (const response of [
    { accountAlias: 'bob', groups: [] },
    {
      accountAlias: 'alice',
      groups: [
        {
          accountAlias: 'bob',
          alias: 'team',
          teamIdHex: '01',
          kind: 'named' as const,
          active: true,
        },
      ],
    },
  ]) {
    let refreshes = 0;
    const bridge = {
      discoverGroups: async () => response,
    } as unknown as Bridge;
    await assert.rejects(
      runTeamDiscovery({
        bridge,
        saved,
        isCurrent: () => true,
        onRefreshSnapshot: async () => {
          refreshes++;
          return FIXTURE;
        },
      }),
      /different account/,
    );
    assert.equal(refreshes, 1);
  }
});

test('old settlement cannot release the newer operation busy owner', () => {
  const owner = ownership();
  const old = owner.capture();
  owner.retire();
  const newer = owner.capture();
  assert.equal(releaseSetupWorkflow(newer, old), newer);
  assert.equal(newer(), true);
  assert.equal(releaseSetupWorkflow(newer, newer), null);
  assert.equal(releaseSetupWorkflow(null, old), null);
});

test('captured ownership includes step, profile name, host, account, mount and retirement', () => {
  const changes: FirstRunCheckpoint[] = [
    { ...saved, state: 'protect' },
    { ...saved, profile: { ...profile, profile: 'other' } },
    { ...saved, profile: { ...profile, hostId: 'other' } },
    { ...saved, account: { ...saved.account!, alias: 'bob' } },
  ];
  for (const next of changes) {
    const owner = ownership();
    const old = owner.capture();
    owner.replace(next);
    assert.equal(old(), false);
    assert.equal(owner.capture()(), true);
  }
  const owner = ownership();
  const old = owner.capture();
  owner.retire();
  const newer = owner.capture();
  assert.equal(old(), false);
  assert.equal(newer(), true);
  owner.unmount();
  assert.equal(newer(), false);
});
