import assert from 'node:assert/strict';
import test from 'node:test';
import { enqueueProfileWork, sharedServerStatus } from '../src/bridge';
import type {
  Bridge,
  CheckedServer,
  ServerStatusSnapshot,
} from '../src/bridge';
import { decodeServers } from '../src/bridge/servers';
import type { AgentSnapshot, Server } from '../src/model';
import {
  readCurrentServerStatus,
  ServerCheckController,
} from '../src/screens/servers/server-workflow';

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

const server = (profileName = 'p'): Server =>
  decodeServers([
    {
      profile_name: profileName,
      display_label: null,
      configured_endpoint: `${profileName}.example`,
      accounts: [],
    },
  ])[0];

const snapshot = (...servers: Server[]): AgentSnapshot =>
  ({
    agent: { state: 'ready' },
    servers,
    observedExpiredLeases: [],
  }) as unknown as AgentSnapshot;

const report = (profile = 'p'): CheckedServer => ({
  profile,
  lookupName: `${profile}.example`,
  canonicalName: `${profile}.example`,
  hostId: 'host',
  chain: 1,
  epoch: 1,
  acceptance: 'inserted',
  serverVersion: null,
});

const status = (profile = 'p'): ServerStatusSnapshot => ({
  profile,
  configuredEndpoint: `${profile}.example`,
  host: null,
  leaseRequired: false,
  leaseExpiresAt: null,
  compatibility: { status: 'not-required' },
  chatSupported: null,
});

const ambiguous = {
  code: 'agent-lost',
  message: 'The result is unknown.',
  retryable: true,
  ambiguous: true,
  fatal: false,
};

function checkHarness(bridge: Bridge) {
  let context = {
    snapshot: snapshot(server()),
    profile: 'p' as string | undefined,
  };
  const events: unknown[] = [];
  const controller = new ServerCheckController(bridge, () => context, {
    busy: (value) => events.push(['busy', value]),
    checked: (_binding, value) => events.push(['checked', value]),
    status: (_binding, value) => events.push(['status', value]),
    toast: (value) => events.push(['toast', value]),
    refresh: async (value) => {
      events.push(['refresh', value]);
    },
    error: async (value) => {
      events.push(['error', value]);
    },
  });
  return {
    controller,
    events,
    replace: (next: typeof context) => {
      context = next;
    },
  };
}

test('status reads check currentness after waiting for profile queue admission', async () => {
  const held = deferred<void>();
  let current = true;
  let reads = 0;
  const bridge = {
    describeServerStatus: async () => {
      reads++;
      return status();
    },
  } as unknown as Bridge;
  const blocker = enqueueProfileWork(bridge, 'p', () => held.promise);
  const pending = readCurrentServerStatus(bridge, server(), () => current);
  current = false;
  held.resolve();
  await blocker;
  assert.equal(await pending, undefined);
  assert.equal(reads, 0);
});

test('current passive status readers share in-flight status without retaining a TTL result', async () => {
  const read = deferred<ServerStatusSnapshot>();
  const started = deferred<void>();
  let reads = 0;
  const bridge = {
    describeServerStatus: () => {
      reads++;
      started.resolve();
      return reads === 1 ? read.promise : Promise.resolve(status());
    },
  } as unknown as Bridge;
  const first = readCurrentServerStatus(bridge, server(), () => true);
  const second = readCurrentServerStatus(bridge, server(), () => true);
  await started.promise;
  const shared = sharedServerStatus(bridge, 'p');
  read.resolve(status());
  await Promise.all([first, second, shared]);
  assert.equal(reads, 1);
  await readCurrentServerStatus(bridge, server(), () => true);
  assert.equal(reads, 2);
});

test('rapid checks dispatch one identity probe and do not automatically reconcile', async () => {
  const read = deferred<CheckedServer>();
  const started = deferred<void>();
  let writes = 0;
  const bridge = {
    checkServer: () => {
      writes++;
      started.resolve();
      return read.promise;
    },
    describeServerStatus: async () => status(),
    reconcileServer: async () =>
      assert.fail('identity check delegated to reconciliation'),
  } as unknown as Bridge;
  const { controller, events } = checkHarness(bridge);
  const first = controller.check(server());
  const duplicate = controller.check(server());
  await started.promise;
  assert.equal(writes, 1);
  read.resolve(report());
  await Promise.all([first, duplicate]);
  assert.equal(writes, 1);
  assert.deepEqual(
    events.map((event) => (event as unknown[])[0]),
    ['busy', 'checked', 'toast', 'status', 'refresh', 'busy'],
  );
});

for (const binding of ['profile', 'address', 'host'] as const) {
  test(`late check errors cannot report or clear busy after ${binding} replacement`, async () => {
    const read = deferred<CheckedServer>();
    const started = deferred<void>();
    const bridge = {
      checkServer: () => {
        started.resolve();
        return read.promise;
      },
    } as unknown as Bridge;
    const { controller, events, replace } = checkHarness(bridge);
    const pending = controller.check(server());
    await started.promise;
    replace({
      snapshot: snapshot({
        ...server(),
        ...(binding === 'address' ? { configuredEndpoint: 'new.example' } : {}),
        ...(binding === 'host' ? { host_id: 'new-host' } : {}),
      }),
      profile: binding === 'profile' ? 'q' : 'p',
    });
    read.reject(ambiguous);
    await pending;
    assert.deepEqual(events, [['busy', true]]);
  });
}

test('retiring a queued check prevents native dispatch', async () => {
  const held = deferred<void>();
  let writes = 0;
  const bridge = {
    checkServer: async () => {
      writes++;
      return report();
    },
  } as unknown as Bridge;
  const blocker = enqueueProfileWork(bridge, 'p', () => held.promise);
  const { controller, events } = checkHarness(bridge);
  const pending = controller.check(server());
  controller.retire();
  held.resolve();
  await blocker;
  await pending;
  assert.equal(writes, 0);
  assert.deepEqual(events, [['busy', true]]);
});

test('old check completion cannot clear a newer attempt after effect reactivation', async () => {
  const first = deferred<CheckedServer>();
  const firstStarted = deferred<void>();
  const second = deferred<CheckedServer>();
  const secondStarted = deferred<void>();
  let writes = 0;
  const bridge = {
    checkServer: () => {
      writes++;
      if (writes === 1) {
        firstStarted.resolve();
        return first.promise;
      }
      secondStarted.resolve();
      return second.promise;
    },
    describeServerStatus: async () => status(),
  } as unknown as Bridge;
  const { controller, events } = checkHarness(bridge);
  const old = controller.check(server());
  await firstStarted.promise;
  controller.retire();
  controller.activate();
  const next = controller.check(server());
  first.reject(ambiguous);
  await old;
  await secondStarted.promise;
  assert.deepEqual(events, [
    ['busy', true],
    ['busy', false],
    ['busy', true],
  ]);
  second.resolve(report());
  await next;
  assert.deepEqual(events.at(-1), ['busy', false]);
});

for (const failed of [false, true]) {
  test(`late post-check status ${failed ? 'errors' : 'results'} cannot reach a new selection`, async () => {
    const read = deferred<ServerStatusSnapshot>();
    const started = deferred<void>();
    const bridge = {
      checkServer: async () => report(),
      describeServerStatus: () => {
        started.resolve();
        return read.promise;
      },
    } as unknown as Bridge;
    const { controller, events, replace } = checkHarness(bridge);
    const pending = controller.check(server());
    await started.promise;
    const before = [...events];
    replace({ snapshot: snapshot(server(), server('q')), profile: 'q' });
    if (failed) read.reject(ambiguous);
    else read.resolve(status());
    await pending;
    assert.deepEqual(events, before);
  });
}

test('fixture-seeded checks require the fixture capability and consume seeding only at dispatch', async () => {
  let writes = 0;
  let seeded = 0;
  const bridge = {
    checkServer: async () => {
      writes++;
      return report();
    },
    describeServerStatus: async () => status(),
  } as unknown as Bridge;
  const production = checkHarness(bridge);
  await production.controller.check(server(), () => {
    seeded++;
  });
  assert.equal(writes, 0);
  assert.equal(seeded, 0);
  const fixture = { ...bridge, fixtureSnapshot: snapshot(server()) };
  const { controller } = checkHarness(fixture);
  const held = deferred<void>();
  const blocker = enqueueProfileWork(fixture, 'p', () => held.promise);
  const old = controller.check(server(), () => {
    seeded++;
  });
  controller.retire();
  controller.activate();
  const next = controller.check(server(), () => {
    seeded++;
  });
  held.resolve();
  await Promise.all([blocker, old, next]);
  assert.equal(writes, 1);
  assert.equal(seeded, 1);
});

test('current ambiguous check failure reports once and never retries', async () => {
  let writes = 0;
  const bridge = {
    checkServer: async () => {
      writes++;
      throw ambiguous;
    },
  } as unknown as Bridge;
  const { controller, events } = checkHarness(bridge);
  await controller.check(server());
  assert.equal(writes, 1);
  assert.equal(
    events.filter((event) => (event as unknown[])[0] === 'error').length,
    1,
  );
  assert.deepEqual(events.at(-1), ['busy', false]);
});
