import assert from 'node:assert/strict';
import test from 'node:test';
import {
  observeProfileWork,
  scheduleProfileWork,
  type WorkTiming,
} from '../src/scheduling/profile-work';
function deferred<T = void>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
const flush = async () => {
  for (let i = 0; i < 10; i++) await Promise.resolve();
};
test('queued foreground work expires without running or releasing active work', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const owner = {},
    gate = deferred();
  const active = scheduleProfileWork(owner, 'p', () => gate.promise);
  await flush();
  const queued = scheduleProfileWork(owner, 'p', async () =>
    assert.fail('expired work ran'),
  );
  const rejected = assert.rejects(queued, {
    code: 'profile-busy',
    ambiguous: false,
  });
  t.mock.timers.tick(75_001);
  await rejected;
  let started = false;
  const next = scheduleProfileWork(owner, 'p', async () => {
    started = true;
  });
  await flush();
  assert.equal(started, false);
  await scheduleProfileWork(owner, 'other', async () => {});
  gate.resolve();
  await Promise.all([active, next]);
  assert.equal(started, true);
});

test('foreground capacity is bounded without blocking independent profiles', async () => {
  const owner = {},
    gate = deferred();
  const active = scheduleProfileWork(owner, 'p', () => gate.promise);
  await flush();
  const waiting = Array.from({ length: 256 }, (_, i) =>
    scheduleProfileWork(owner, 'p', async () => i),
  );
  await assert.rejects(
    scheduleProfileWork(owner, 'p', async () =>
      assert.fail('excess request ran'),
    ),
    { code: 'profile-busy', ambiguous: false },
  );
  await scheduleProfileWork(owner, 'other', async () => {});
  gate.resolve();
  await active;
  assert.equal((await Promise.all(waiting)).length, 256);
});

test('default work is asynchronous FIFO per profile and independent across profiles and owners', async () => {
  const owner = {},
    other = {},
    gate = deferred(),
    order: string[] = [];
  let active = 0,
    peak = 0;
  const first = scheduleProfileWork(owner, 'p', async () => {
    order.push('first');
    peak = Math.max(peak, ++active);
    await gate.promise;
    active--;
  });
  const second = scheduleProfileWork(owner, 'p', async () => {
    order.push('second');
    peak = Math.max(peak, ++active);
    active--;
    return 2;
  });
  assert.equal(order.length, 0);
  await Promise.all([
    scheduleProfileWork(owner, 'q', async () => {
      order.push('other profile');
    }),
    scheduleProfileWork(other, 'p', async () => {
      order.push('other owner');
    }),
  ]);
  assert.deepEqual(order, ['first', 'other profile', 'other owner']);
  gate.resolve();
  await first;
  assert.equal(await second, 2);
  assert.equal(peak, 1);
});
test('errors and observers cannot poison the queue; idle and reentrant submissions remain ordered', async () => {
  const owner = {},
    order: number[] = [],
    events: unknown[] = [];
  const off = observeProfileWork(owner, (e) => {
    events.push(e);
    throw Error('observer');
  });
  const failed = scheduleProfileWork(owner, 'p', async () => {
    throw Error('operation');
  });
  await assert.rejects(failed, /operation/);
  let nested: Promise<void> | undefined;
  await scheduleProfileWork(owner, 'p', async () => {
    order.push(1);
    nested = scheduleProfileWork(owner, 'p', async () => {
      order.push(3);
    });
    order.push(2);
  });
  await nested;
  await flush();
  await scheduleProfileWork(owner, 'p', async () => {
    order.push(4);
  });
  assert.deepEqual(order, [1, 2, 3, 4]);
  assert.equal(events.length, 4);
  assert.deepEqual(Object.keys(events[0] as object).sort(), [
    'busy',
    'executionMilliseconds',
    'outcome',
    'priority',
    'profile',
    'queueMilliseconds',
  ]);
  assert.equal((events[0] as { profile: string }).profile, 'p');
  off();
});

test('a foreground timing carries the label its caller gave the work', async () => {
  const owner = {},
    events: WorkTiming[] = [];
  const off = observeProfileWork(owner, (event) => {
    events.push(event);
  });
  await scheduleProfileWork(
    owner,
    'srv-1',
    async () => 1,
    undefined,
    'profile-catalog',
  );
  await scheduleProfileWork(owner, 'srv-2', async () => 2);
  assert.deepEqual(
    events.map((event) => [
      event.profile,
      event.key,
      event.priority,
      event.outcome,
    ]),
    [
      ['srv-1', 'profile-catalog', 'foreground', 'success'],
      // Work whose caller named nothing carries no key at all.
      ['srv-2', undefined, 'foreground', 'success'],
    ],
  );
  off();
});

function background(key: string, controller = new AbortController()) {
  return {
    key,
    owner: {},
    generation: 1,
    signal: controller.signal,
    current: () => true,
    cancel: () => {},
    preemptible: false as const,
  };
}
test('foreground overtakes queued history, but waits for the running request to settle', async () => {
  const owner = {},
    gate = deferred(),
    order: string[] = [];
  const first = scheduleProfileWork(
    owner,
    'p',
    async () => {
      order.push('page1');
      await gate.promise;
    },
    background('a'),
  );
  await flush();
  const second = scheduleProfileWork(
    owner,
    'p',
    async () => {
      order.push('page2');
    },
    background('b'),
  );
  const explicit1 = scheduleProfileWork(owner, 'p', async () => {
    order.push('user1');
  });
  const explicit2 = scheduleProfileWork(owner, 'p', async () => {
    order.push('user2');
  });
  await flush();
  assert.deepEqual(order, ['page1']);
  gate.resolve();
  await Promise.all([first, second, explicit1, explicit2]);
  assert.deepEqual(order, ['page1', 'user1', 'user2', 'page2']);
});
test('two-page pass reacquires background admission and yields to foreground', async () => {
  const owner = {},
    gate = deferred(),
    order: string[] = [];
  const options = background('channel');
  const pass = (async () => {
    await scheduleProfileWork(
      owner,
      'p',
      async () => {
        order.push('first');
        await gate.promise;
      },
      options,
    );
    await scheduleProfileWork(
      owner,
      'p',
      async () => {
        order.push('second');
      },
      options,
    );
  })();
  await flush();
  const foreground = scheduleProfileWork(owner, 'p', async () => {
    order.push('user');
  });
  gate.resolve();
  await Promise.all([pass, foreground]);
  assert.deepEqual(order, ['first', 'user', 'second']);
});
test('queued duplicates share one result, cancellation removes them, and stale owners never run', async () => {
  const owner = {},
    gate = deferred(),
    controller = new AbortController();
  const hold = scheduleProfileWork(owner, 'p', () => gate.promise);
  const options = background('channel', controller);
  const one = scheduleProfileWork(
    owner,
    'p',
    async () => assert.fail('cancelled'),
    options,
  );
  const two = scheduleProfileWork(
    owner,
    'p',
    async () => assert.fail('duplicate'),
    options,
  );
  assert.equal(one, two);
  controller.abort();
  await assert.rejects(one, { code: 'cancelled' });
  let current = true;
  const stale = scheduleProfileWork(
    owner,
    'p',
    async () => assert.fail('stale'),
    { ...background('other'), current: () => current },
  );
  current = false;
  gate.resolve();
  await hold;
  await assert.rejects(stale, { code: 'cancelled' });
});
test('active lifecycle cancellation waits for settlement before releasing the profile', async () => {
  const owner = {},
    gate = deferred(),
    controller = new AbortController();
  let cancelled = 0,
    started = false;
  const running = scheduleProfileWork(owner, 'p', () => gate.promise, {
    ...background('channel', controller),
    cancel: () => {
      cancelled++;
    },
  });
  await flush();
  controller.abort();
  controller.abort();
  const next = scheduleProfileWork(owner, 'p', async () => {
    started = true;
  });
  await flush();
  assert.equal(started, false);
  assert.equal(cancelled, 1);
  gate.resolve();
  await assert.rejects(running, { code: 'cancelled' });
  await next;
});
test('background capacity rejects excess work without poisoning healthy admission', async () => {
  const owner = {},
    gate = deferred();
  const hold = scheduleProfileWork(owner, 'p', () => gate.promise);
  const queued = Array.from({ length: 64 }, (_, i) =>
    scheduleProfileWork(owner, 'p', async () => i, background(String(i))),
  );
  await assert.rejects(
    scheduleProfileWork(owner, 'p', async () => 65, background('overflow')),
    { code: 'profile-busy' },
  );
  gate.resolve();
  await hold;
  assert.equal((await Promise.all(queued)).length, 64);
});

test('background rotation serves other identities before a repeatedly resubmitted key', async () => {
  const owner = {},
    gate = deferred(),
    order: string[] = [];
  const a = background('a'),
    b = background('b');
  const first = scheduleProfileWork(owner, 'p', () => gate.promise, a);
  await flush();
  const again = scheduleProfileWork(
    owner,
    'p',
    async () => {
      order.push('a');
    },
    a,
  );
  const other = scheduleProfileWork(
    owner,
    'p',
    async () => {
      order.push('b');
      throw Error('retryable');
    },
    b,
  );
  const failed = assert.rejects(other, /retryable/);
  gate.resolve();
  await Promise.all([first, again, failed]);
  assert.deepEqual(order, ['b', 'a']);
});

test('work queued behind a request that uses its whole budget runs once the queue frees', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const owner = {},
    gate = deferred();
  const active = scheduleProfileWork(owner, 'p', () => gate.promise);
  await flush();
  let ran = false;
  const queued = scheduleProfileWork(owner, 'p', async () => {
    ran = true;
  });
  // The agent's own request budget: the active request settles as it ends.
  t.mock.timers.tick(60_000);
  gate.resolve();
  await active;
  await queued;
  assert.equal(ran, true);
});

test('the chat lane is admitted while the default lane is busy, and stays serialized', async () => {
  const owner = {},
    gate = deferred(),
    chatGate = deferred(),
    order: string[] = [];
  let concurrent = 0,
    peak = 0;
  const active = scheduleProfileWork(owner, 'p', async () => {
    order.push('default');
    await gate.promise;
  });
  await flush();
  const chat = scheduleProfileWork(
    owner,
    'p',
    async () => {
      order.push('chat-1');
      peak = Math.max(peak, ++concurrent);
      await chatGate.promise;
      concurrent--;
    },
    undefined,
    'chat:history',
    'chat',
  );
  const queued = scheduleProfileWork(
    owner,
    'p',
    async () => {
      order.push('chat-2');
      peak = Math.max(peak, ++concurrent);
      concurrent--;
    },
    undefined,
    'chat:channels',
    'chat',
  );
  await flush();
  // The chat request did not wait for the default lane's work to settle.
  assert.deepEqual(order, ['default', 'chat-1']);
  chatGate.resolve();
  await queued;
  assert.deepEqual(order, ['default', 'chat-1', 'chat-2']);
  // Chat work is still one request at a time.
  assert.equal(peak, 1);
  gate.resolve();
  await Promise.all([active, chat]);
});

test('a chat lane timing reports its lane and its own wait', async (t) => {
  let now = 0;
  t.mock.method(performance, 'now', () => now);
  const owner = {},
    gate = deferred(),
    chatGate = deferred(),
    events: WorkTiming[] = [];
  const off = observeProfileWork(owner, (event) => {
    events.push(event);
  });
  const active = scheduleProfileWork(owner, 'p', () => gate.promise);
  await flush();
  now = 100;
  const first = scheduleProfileWork(
    owner,
    'p',
    () => chatGate.promise,
    undefined,
    'chat:history',
    'chat',
  );
  const second = scheduleProfileWork(
    owner,
    'p',
    async () => undefined,
    undefined,
    'chat:pending',
    'chat',
  );
  // Admission overhead and time waiting behind active work are explicit,
  // independent of CPU scheduling and the real clock's resolution.
  now = 105;
  await flush();
  now = 125;
  chatGate.resolve();
  await Promise.all([first, second]);
  gate.resolve();
  await active;
  off();
  assert.deepEqual(
    events.map((event) => [event.profile, event.key, event.priority]),
    [
      ['p', 'chat:history', 'chat'],
      ['p', 'chat:pending', 'chat'],
      ['p', undefined, 'foreground'],
    ],
  );
  // Each wait starts at that request's enqueue time, in its own lane.
  assert.deepEqual(
    events.map((event) => event.queueMilliseconds),
    [5, 25, 0],
  );
});

test('each lane bounds its own capacity and the profile outlives one lane draining', async () => {
  const owner = {},
    gate = deferred(),
    chatGate = deferred();
  const active = scheduleProfileWork(owner, 'p', () => gate.promise);
  const chat = scheduleProfileWork(
    owner,
    'p',
    () => chatGate.promise,
    undefined,
    'chat:history',
    'chat',
  );
  await flush();
  const waiting = Array.from({ length: 256 }, (_, i) =>
    scheduleProfileWork(owner, 'p', async () => i),
  );
  // A full default lane does not refuse chat work.
  const admitted = scheduleProfileWork(
    owner,
    'p',
    async () => 'chat',
    undefined,
    'chat:channels',
    'chat',
  );
  await assert.rejects(
    scheduleProfileWork(owner, 'p', async () =>
      assert.fail('excess request ran'),
    ),
    { code: 'profile-busy', ambiguous: false },
  );
  chatGate.resolve();
  assert.equal(await admitted, 'chat');
  // The chat lane drained first; the default lane's queue survived it.
  gate.resolve();
  await Promise.all([active, chat]);
  assert.equal((await Promise.all(waiting)).length, 256);
});
