import assert from 'node:assert/strict';
import test from 'node:test';
import type { Bridge } from '../src/bridge';
import type {
  ChatAction,
  ChatMessage,
  ChatReply,
  ChatScope,
} from '../src/chat-contract';
import type {
  ChatClock,
  ChatInboxService,
  TeamInbox,
} from '../src/chat/inbox-service';
import {
  NotificationConsumer,
  type NotificationMetric,
} from '../src/chat/notification-consumer';
import {
  classifyNotificationChange,
  notificationView,
} from '../src/chat/notification-changes';
import { notificationKey } from '../src/chat/local-contract';
import type { LocalSession } from '../src/chat/local-contract';

Object.defineProperty(globalThis, 'document', {
  value: { visibilityState: 'hidden', hasFocus: () => false },
  configurable: true,
});
class Clock implements ChatClock {
  time = 0;
  next = 0;
  timers = new Map<number, { due: number; fn(): void }>();
  now = () => this.time;
  random = () => 0;
  later = (fn: () => void, delay: number) => {
    const id = ++this.next;
    this.timers.set(id, { due: this.time + delay, fn });
    return id;
  };
  cancel = (timer: unknown) => {
    this.timers.delete(timer as number);
  };
  run() {
    for (const [id, timer] of [...this.timers])
      if (timer.due <= this.time && this.timers.delete(id)) timer.fn();
  }
  async flush(rounds = 50) {
    for (let i = 0; i < rounds; i++) {
      this.run();
      // Preference hashing uses WebCrypto's worker pool. Drain a crypto task as
      // well as JS microtasks; a fixed number of setImmediate calls can race it.
      await crypto.subtle.digest('SHA-256', new Uint8Array());
      await new Promise<void>((r) => setImmediate(r));
    }
  }
  async advance(ms: number) {
    this.time += ms;
    await this.flush();
  }
}
const scope: ChatScope = {
  host: 'host',
  actor: 'me',
  store: { profile: 'p', account_alias: 'a', team_alias: 't', team_id: 'team' },
};
const message = (sequence: number): ChatMessage => ({
  id: `message-${sequence}`,
  sequence: String(sequence),
  sender: 'other',
  send_time: '1',
  insert_time: '1',
  content: { kind: 'text', text: 'private text' },
});
function entry(ids = ['a', 'b'], selectedScope = scope): TeamInbox {
  const channels = ids.map((id) => ({
    id,
    readable: true,
    writable: true,
    admin: true,
    name: '',
    description: null,
    read_role: 'member',
    write_role: 'member',
  }));
  return {
    state: 'ready',
    scope: selectedScope,
    stale: false,
    error: '',
    note: '',
    revision: 1,
    authorizationRevision: 1,
    channelRevisions: new Map(ids.map((id) => [id, 1])),
    blockedChannels: new Set(),
    data: {
      kind: 'inbox',
      channels,
      conversations: channels.map((channel) => ({
        channel,
        muted: false,
        hidden: false,
        unread: '0',
        preview: null,
        read_through: '0',
        pending_read: null,
        inbox_version: '1',
      })),
      degraded: false,
      previews_incomplete: false,
      read_retry_pending: false,
      blocked_channels: [],
      cursor: '1',
      head: '1',
    },
  };
}
function setup(initial = entry()) {
  const clock = new Clock(),
    events: NotificationMetric[] = [],
    calls: string[] = [],
    alerts: unknown[] = [];
  let snapshot = new Map([['store', initial]]);
  const listeners = new Set<() => void>();
  const rows = new Map<string, ChatMessage[]>();
  let history: (
    id: string,
    action: Extract<ChatAction, { action: 'notification-history' }>,
  ) => Promise<ChatReply> = async (id, action) => ({
    scope: snapshot.get(id)!.scope!,
    result: {
      kind: 'history',
      channel: action.channel,
      messages: rows.get(action.channel) ?? [message(1)],
      before: null,
      missing_predecessors: [],
    },
  });
  const service = {
    getSnapshot: () => snapshot,
    subscribe: (fn: () => void) => {
      listeners.add(fn);
      return () => listeners.delete(fn);
    },
    invalidate: () => {},
    blockChannel: (_id: string, channel: string) =>
      update({ blockedChannels: new Set([channel]) }),
    block: () => update({ state: 'blocked' }),
    handleError: (_id: string, cause: unknown) => {
      if ((cause as { code?: string }).code !== 'chat-integrity') return false;
      update({ state: 'blocked' });
      return true;
    },
  } as unknown as ChatInboxService;
  const bridge = {
    chat: async (id: string, action: ChatAction) => {
      assert.equal(action.action, 'notification-history');
      calls.push(action.channel);
      return history(id, action);
    },
    cancelChat: async () => {},
    chatLocal: async (action: unknown) => {
      alerts.push(action);
    },
  } as unknown as Bridge;
  const session: LocalSession = {
    epoch: 'aa'.repeat(16),
    available: true,
    settings: { enabled: true, previews: false, overrides: {} },
  };
  function update(patch: Partial<TeamInbox>) {
    snapshot = new Map([['store', { ...snapshot.get('store')!, ...patch }]]);
    for (const listener of listeners) listener();
  }
  const make = () =>
    new NotificationConsumer(
      bridge,
      service,
      session,
      () => {},
      clock,
      (e) => events.push(e),
    );
  return {
    clock,
    calls,
    alerts,
    events,
    rows,
    session,
    bridge,
    service,
    make,
    update,
    get: () => snapshot.get('store')!,
    replace: (next: Map<string, TeamInbox>) => {
      snapshot = next;
      for (const listener of listeners) listener();
    },
    history: (fn: typeof history) => {
      history = fn;
    },
  };
}
test('classifier separates content, policy, preference, freshness and scope changes', () => {
  const e = entry(),
    a = notificationView(e, 'a')!;
  const status = classifyNotificationChange(
    a,
    notificationView({ ...e, error: 'retry status', revision: 999 }, 'a'),
  );
  assert.equal(status.content, false);
  assert.equal(status.baseline, false);
  assert.equal(
    classifyNotificationChange(a, { ...a, revision: 2 }).content,
    true,
  );
  assert.equal(
    classifyNotificationChange(a, { ...a, muted: true, eligible: false })
      .policy,
    true,
  );
  assert.equal(
    classifyNotificationChange(a, { ...a, preference: false }).preference,
    true,
  );
  assert.equal(
    classifyNotificationChange(a, { ...a, fresh: false }).freshness,
    true,
  );
  assert.equal(
    classifyNotificationChange(a, {
      ...a,
      scope: { ...scope, actor: 'replacement' },
    }).baseline,
    true,
  );
  assert.equal(classifyNotificationChange(a, undefined).membership, true);
});
test('only changed channels fetch, repeated changes coalesce, and status-only updates do not fetch', async () => {
  const f = setup(),
    c = f.make();
  try {
    await f.clock.flush();
    assert.deepEqual(f.calls, ['a', 'b']);
    assert.equal(f.alerts.length, 0);
    f.update({ error: 'status only', revision: 2 });
    await f.clock.flush();
    assert.equal(f.calls.length, 2);
    f.rows.set('b', [message(2), message(1)]);
    f.update({
      channelRevisions: new Map([
        ['a', 1],
        ['b', 2],
      ]),
    });
    f.update({
      channelRevisions: new Map([
        ['a', 1],
        ['b', 3],
      ]),
    });
    await f.clock.flush();
    assert.deepEqual(f.calls, ['a', 'b', 'b']);
    assert.equal(f.alerts.length, 1);
  } finally {
    c.stop();
  }
});
test('revision arriving during fetch survives the captured commit', async () => {
  const f = setup(entry(['a'])),
    c = f.make();
  try {
    await f.clock.flush();
    let resolve!: (reply: ChatReply) => void;
    f.history(
      async () =>
        new Promise((r) => {
          resolve = r;
        }),
    );
    f.update({ channelRevisions: new Map([['a', 2]]) });
    await f.clock.flush();
    f.update({ channelRevisions: new Map([['a', 3]]) });
    await f.clock.flush();
    f.history(async () => ({
      scope,
      result: {
        kind: 'history',
        channel: 'a',
        messages: [message(3), message(2)],
        before: null,
        missing_predecessors: [],
      },
    }));
    resolve({
      scope,
      result: {
        kind: 'history',
        channel: 'a',
        messages: [message(2)],
        before: null,
        missing_predecessors: [],
      },
    });
    await f.clock.flush();
    assert.equal(f.calls.length, 3);
    assert.equal(
      f.events.filter((e) => e.kind === 'pass' && !e.baselineOnly).length,
      2,
    );
  } finally {
    c.stop();
  }
});
test('overflow rotates beyond 64 entries and retry backoff is local to the failed channel', async () => {
  const f = setup(entry(Array.from({ length: 130 }, (_, i) => String(i))));
  f.history(async (_id, action) => {
    if (action.channel === '0')
      throw {
        code: 'offline',
        message: 'offline',
        retryable: true,
        fatal: false,
        ambiguous: false,
      };
    return {
      scope,
      result: {
        kind: 'history',
        channel: action.channel,
        messages: [],
        before: null,
        missing_predecessors: [],
      },
    };
  });
  const c = f.make();
  try {
    for (let i = 0; i < 100 && new Set(f.calls).size < 130; i++)
      await f.clock.flush();
    assert.equal(new Set(f.calls).size, 130);
    assert.equal(f.calls.filter((x) => x === '0').length, 1);
    await f.clock.advance(999);
    assert.equal(f.calls.filter((x) => x === '0').length, 1);
    await f.clock.advance(1);
    assert.equal(f.calls.filter((x) => x === '0').length, 2);
    for (const e of f.events)
      if (e.kind === 'state') {
        assert.ok(e.queued <= 64);
        assert.ok(e.active <= 2);
        assert.ok(e.baselines <= 4096);
      }
  } finally {
    c.stop();
  }
});
test('local disable remains suppressed without rebaseline loops and reenable starts fresh', async () => {
  const f = setup(entry(['a']));
  f.session.settings.overrides[await notificationKey(scope, 'a')] = false;
  const c = f.make();
  try {
    await f.clock.flush();
    await f.clock.advance(30000);
    assert.equal(f.calls.length, 0);
    f.session.settings.overrides[await notificationKey(scope, 'a')] = true;
    f.update({ revision: 2 });
    await f.clock.flush();
    assert.equal(f.calls.length, 1);
    assert.equal(
      f.events.filter((e) => e.kind === 'pass' && !e.baselineOnly).length,
      0,
    );
  } finally {
    c.stop();
  }
});
test('access loss waits for new authorized projection, then starts a fresh baseline', async () => {
  const f = setup(entry(['a'])),
    c = f.make();
  try {
    await f.clock.flush();
    f.history(async () => {
      throw {
        code: 'chat-access-denied',
        message: 'denied',
        fatal: false,
        ambiguous: false,
        retryable: false,
      };
    });
    f.update({ channelRevisions: new Map([['a', 2]]) });
    await f.clock.flush();
    await f.clock.advance(30000);
    assert.equal(f.calls.length, 2);
    f.history(async () => ({
      scope,
      result: {
        kind: 'history',
        channel: 'a',
        messages: [message(8)],
        before: null,
        missing_predecessors: [],
      },
    }));
    f.update({ authorizationRevision: 2 });
    await f.clock.flush();
    assert.equal(f.calls.length, 3);
    assert.equal(
      f.events.filter((e) => e.kind === 'pass' && !e.baselineOnly).length,
      0,
    );
  } finally {
    c.stop();
  }
});
test('degraded fallback is timed, stays incomplete, and unread/preview hints cannot produce candidates', async () => {
  const initial = entry(['a']);
  initial.data = { ...initial.data!, degraded: true };
  const f = setup(initial),
    c = f.make();
  try {
    await f.clock.flush();
    f.update({ revision: 20 });
    await f.clock.flush();
    assert.equal(f.calls.length, 1);
    await f.clock.advance(4999);
    assert.equal(f.calls.length, 1);
    await f.clock.advance(1);
    assert.equal(f.calls.length, 2);
    assert.equal(f.alerts.length, 0);
    for (const e of f.events)
      if (e.kind === 'pass') assert.equal(e.incomplete, true);
  } finally {
    c.stop();
  }
});
test('scope replacement and shutdown reject late results and do not release unsettled profile work', async () => {
  const f = setup(entry(['a']));
  let resolve!: (value: ChatReply) => void;
  f.history(
    async () =>
      new Promise((r) => {
        resolve = r;
      }),
  );
  const first = f.make();
  await f.clock.flush();
  first.stop();
  const next = f.make();
  await f.clock.flush();
  assert.equal(f.calls.length, 1);
  f.history(async () => ({
    scope,
    result: {
      kind: 'history',
      channel: 'a',
      messages: [message(7)],
      before: null,
      missing_predecessors: [],
    },
  }));
  resolve({
    scope,
    result: {
      kind: 'history',
      channel: 'a',
      messages: [message(6)],
      before: null,
      missing_predecessors: [],
    },
  });
  await f.clock.flush();
  assert.equal(f.calls.length, 2);
  assert.equal(f.alerts.length, 0);
  f.update({ scope: { ...scope, actor: 'replacement' } });
  f.history(async () => ({
    scope: f.get().scope!,
    result: {
      kind: 'history',
      channel: 'a',
      messages: [message(99)],
      before: null,
      missing_predecessors: [],
    },
  }));
  await f.clock.flush();
  assert.equal(f.calls.length, 3);
  assert.equal(f.alerts.length, 1); // old alerts cleared, no display
  assert.equal((f.alerts[0] as { action: string }).action, 'clear');
  next.stop();
});
test('two-page catch-up yields to foreground and commits a fixed upper bound', async () => {
  const { enqueueProfileWork } = await import('../src/bridge');
  const f = setup(entry(['a'])),
    c = f.make();
  try {
    await f.clock.flush();
    const order: string[] = [];
    let foreground: Promise<unknown> | undefined;
    f.history(async (_id, action) => {
      order.push(action.before === null ? 'first' : 'second');
      if (action.before === null) {
        foreground = enqueueProfileWork(f.bridge, 'p', async () => {
          order.push('foreground');
        });
        return {
          scope,
          result: {
            kind: 'history',
            channel: 'a',
            messages: [message(5)],
            before: '5',
            missing_predecessors: [],
          },
        };
      }
      return {
        scope,
        result: {
          kind: 'history',
          channel: 'a',
          messages: [message(6), message(4), message(1)],
          before: null,
          missing_predecessors: [],
        },
      };
    });
    f.update({ channelRevisions: new Map([['a', 2]]) });
    await f.clock.flush();
    await foreground;
    assert.deepEqual(order, ['first', 'foreground', 'second']);
    const pass = f.events.find((e) => e.kind === 'pass' && !e.baselineOnly);
    assert.ok(pass?.kind === 'pass');
    assert.deepEqual(
      new Set(pass.candidates),
      new Set(['message-4', 'message-5']),
    );
  } finally {
    c.stop();
  }
});
for (const policy of [
  'muted',
  'hidden',
  'unreadable',
  'quarantined',
  'stale',
] as const) {
  test(`${policy} cancels eligibility and recovery establishes a fresh baseline`, async () => {
    const f = setup(entry(['a'])),
      c = f.make();
    try {
      await f.clock.flush();
      const data = structuredClone(f.get().data!);
      if (policy === 'muted' || policy === 'hidden')
        data.conversations[0][policy] = true;
      if (policy === 'unreadable') data.channels[0].readable = false;
      f.update({
        data,
        stale: policy === 'stale',
        blockedChannels: new Set(policy === 'quarantined' ? ['a'] : []),
        channelRevisions: new Map([['a', 2]]),
      });
      await f.clock.flush();
      assert.equal(f.calls.length, 1);
      f.rows.set('a', [message(99)]);
      f.update(entry(['a']));
      await f.clock.flush();
      assert.equal(f.calls.length, 2);
      assert.equal(
        f.events.filter((e) => e.kind === 'pass' && !e.baselineOnly).length,
        0,
      );
    } finally {
      c.stop();
    }
  });
}
test('admission counts unsettled requests globally and once per profile', async () => {
  const { NotificationAdmission } =
    await import('../src/chat/notification-admission');
  const admission = new NotificationAdmission();
  const a = admission.acquire('a')!,
    b = admission.acquire('b')!;
  assert.equal(admission.active, 2);
  assert.equal(admission.acquire('a'), undefined);
  assert.equal(admission.acquire('c'), undefined);
  a();
  a();
  const c = admission.acquire('c')!;
  assert.equal(admission.active, 2);
  b();
  c();
  assert.equal(admission.active, 0);
});
test('4096 baseline bound rotates with fresh baselines and a bounded eviction rate', async () => {
  const f = setup(entry(['a']));
  const snapshot = new Map<string, TeamInbox>();
  for (let i = 0; i < 17; i++)
    snapshot.set(
      `store${i}`,
      entry(
        Array.from({ length: i === 16 ? 1 : 256 }, (_, n) => `${i}-${n}`),
        { ...scope, store: { ...scope.store, team_id: `team${i}` } },
      ),
    );
  f.replace(snapshot);
  const c = f.make();
  try {
    for (
      let i = 0;
      i < 3000 &&
      f.events.filter((e) => e.kind === 'pass' && !e.budgetHit).length < 4160;
      i++
    )
      await f.clock.advance(1);
    await f.clock.flush();
    assert.equal(f.events.filter((e) => e.kind === 'eviction').length, 64);
    assert.equal(
      f.events.filter((e) => e.kind === 'pass' && !e.baselineOnly).length,
      0,
    );
    assert.ok(f.events.some((e) => e.kind === 'state' && e.baselines === 4096));
    assert.ok(
      f.events.every(
        (e) => e.kind !== 'state' || (e.baselines <= 4096 && e.queued <= 64),
      ),
    );
    const calls = f.calls.length;
    await f.clock.flush();
    assert.equal(f.calls.length, calls);
    await f.clock.advance(5000);
    assert.ok(f.calls.length > calls);
  } finally {
    c.stop();
  }
});
test('first-reply scope mismatch blocks the shared owner without quarantining channel contents', async () => {
  const f = setup(entry(['a']));
  f.history(async () => ({
    scope: { ...scope, actor: 'replacement' },
    result: {
      kind: 'history',
      channel: 'a',
      messages: [message(2)],
      before: null,
      missing_predecessors: [],
    },
  }));
  const c = f.make();
  try {
    await f.clock.flush();
    assert.equal(f.get().state, 'blocked');
    assert.equal(f.get().blockedChannels.size, 0);
    assert.equal(f.events.filter((e) => e.kind === 'pass').length, 0);
  } finally {
    c.stop();
  }
});
test('read-authority changes establish fresh baselines while the channel stays readable', async () => {
  const f = setup(entry(['a'])),
    c = f.make();
  try {
    await f.clock.flush();
    const previous = notificationView(f.get(), 'a')!;
    f.rows.set('a', [message(100)]);
    const data = structuredClone(f.get().data!);
    data.channels[0].read_role = 'admin';
    const next = notificationView({ ...f.get(), data }, 'a')!;
    const change = classifyNotificationChange(previous, next);
    assert.equal(change.authorization, true);
    assert.equal(change.baseline, true);
    assert.equal(change.content, false);
    f.update({ data, channelRevisions: new Map([['a', 2]]) });
    await f.clock.flush();
    assert.equal(f.calls.length, 2);
    assert.equal(
      f.events.filter((e) => e.kind === 'pass' && !e.baselineOnly).length,
      0,
    );
  } finally {
    c.stop();
  }
});
test('authority change during history rejects publication before the next scan', async () => {
  const f = setup(entry(['a'])),
    c = f.make();
  try {
    await f.clock.flush();
    let complete!: (value: ChatReply) => void;
    f.history(
      () =>
        new Promise((resolve) => {
          complete = resolve;
        }),
    );
    f.update({ channelRevisions: new Map([['a', 2]]) });
    await f.clock.flush();
    const data = structuredClone(f.get().data!);
    data.channels[0].admin = false;
    f.update({ data });
    complete({
      scope,
      result: {
        kind: 'history',
        channel: 'a',
        messages: [message(2)],
        before: null,
        missing_predecessors: [],
      },
    });
    // Do not run the scheduled snapshot scan. Publication must check current authority itself.
    await new Promise<void>((resolve) => setImmediate(resolve));
    assert.equal(
      f.events.filter((e) => e.kind === 'pass' && !e.baselineOnly).length,
      0,
    );
    assert.equal(f.alerts.length, 0);
  } finally {
    c.stop();
  }
});
test('regressed history cannot rewind a known baseline and replay old alerts', async () => {
  const f = setup(entry(['a']));
  f.rows.set('a', [message(10)]);
  const c = f.make();
  try {
    await f.clock.flush();
    f.rows.set('a', [message(3)]);
    f.update({ channelRevisions: new Map([['a', 2]]) });
    await f.clock.flush();
    f.rows.set('a', [message(8)]);
    f.update({ channelRevisions: new Map([['a', 3]]) });
    await f.clock.flush();
    assert.equal(f.alerts.length, 0);
    assert.equal(
      f.events.filter((e) => e.kind === 'pass' && e.incomplete).length,
      2,
    );
    f.rows.set('a', [message(10)]);
    await f.clock.advance(4999);
    assert.equal(f.calls.length, 3);
    await f.clock.advance(1);
    assert.equal(f.calls.length, 4);
    assert.equal(f.alerts.length, 0);
    f.rows.set('a', [message(11)]);
    f.update({ channelRevisions: new Map([['a', 4]]) });
    await f.clock.flush();
    assert.deepEqual(
      f.events.flatMap((e) => (e.kind === 'pass' ? [...e.candidates] : [])),
      ['message-11'],
    );
  } finally {
    c.stop();
  }
});

test('the pass budget counts baseline pages and serves leftovers before newly dirty channels', async () => {
  const ids = Array.from({ length: 12 }, (_, i) => `channel-${i}`);
  const f = setup(entry(ids));
  for (const id of ids)
    f.rows.set(
      id,
      Array.from({ length: 50 }, (_, i) => message(50 - i)),
    );
  const c = f.make();
  try {
    await f.clock.flush();
    const first = [...f.calls];
    assert.ok(first.length > 0 && first.length < ids.length);
    assert.ok(first.length * 50 <= 400);
    const summary = f.events.find((e) => e.kind === 'pass' && e.budgetHit);
    assert.ok(summary?.kind === 'pass');
    assert.equal(summary.rows, first.length * 50);
    assert.equal(
      summary.bytes,
      first.length * 50 * new TextEncoder().encode('private text').length,
    );
    f.update({ channelRevisions: new Map(ids.map((id) => [id, 2])) });
    await f.clock.flush();
    assert.deepEqual(f.calls, first);
    await f.clock.advance(1);
    assert.deepEqual(
      f.calls.slice(first.length, ids.length),
      ids.slice(first.length),
    );
    assert.equal(new Set(f.calls).size, ids.length);
  } finally {
    c.stop();
  }
});

test('two-page notification reads stay within the pass allowance without dropping deferred alerts', async () => {
  const ids = Array.from({ length: 8 }, (_, i) => `c${i}`);
  const f = setup(entry(ids));
  const c = f.make();
  try {
    await f.clock.flush();
    f.calls.length = 0;
    f.events.length = 0;
    f.history(async (_id, action) => {
      const high = action.before === null ? 101 : 51;
      return {
        scope,
        result: {
          kind: 'history',
          channel: action.channel,
          messages: Array.from({ length: 50 }, (_, i) => message(high - i)),
          before: String(high - 49),
          missing_predecessors: [],
        },
      };
    });
    f.update({ channelRevisions: new Map(ids.map((id) => [id, 2])) });
    await f.clock.flush();
    assert.equal(f.calls.length, 8);
    assert.deepEqual(
      f.calls,
      ids.slice(0, 4).flatMap((id) => [id, id]),
    );
    assert.ok(
      f.events.some((e) => e.kind === 'pass' && e.budgetHit && e.rows === 400),
    );
    await f.clock.advance(1);
    assert.deepEqual(
      f.calls,
      ids.flatMap((id) => [id, id]),
    );
    assert.equal(
      f.events.filter((e) => e.kind === 'pass' && !e.baselineOnly).length,
      8,
    );
  } finally {
    c.stop();
  }
});
