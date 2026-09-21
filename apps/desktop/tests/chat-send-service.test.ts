import assert from 'node:assert/strict';
import test from 'node:test';
import { ChatSendService, type ChatSendTiming } from '../src/chat/send-service';
import { FIXTURE } from '../src/fixture';
import { mockBridge } from '../src/mock-bridge';
import type { ChatAction, ChatReply } from '../src/chat-contract';
import type { ChatClock, TeamInbox } from '../src/chat/inbox-service';

const settle = () => new Promise<void>((resolve) => setImmediate(resolve));

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

/** Time only moves when a test moves it, so an idle wait is observable. */
class Clock implements ChatClock {
  time = Date.now();
  next = 0;
  timers = new Map<number, { due: number; fn: () => void }>();
  now = () => this.time;
  random = () => 0.5;
  later = (fn: () => void, delay: number) => {
    const id = ++this.next;
    this.timers.set(id, { due: this.time + delay, fn });
    return id;
  };
  cancel = (id: unknown) => {
    this.timers.delete(id as number);
  };
  async advance(milliseconds: number) {
    const end = this.time + milliseconds;
    for (;;) {
      const next = [...this.timers]
        .filter(([, timer]) => timer.due <= end)
        .sort((a, b) => a[1].due - b[1].due)[0];
      if (!next) break;
      this.time = next[1].due;
      this.timers.delete(next[0]);
      next[1].fn();
      await settle();
    }
    this.time = end;
    await settle();
  }
}
async function setup(
  intercept?: (
    action: ChatAction,
    run: () => Promise<ChatReply>,
  ) => Promise<ChatReply>,
) {
  const snapshot = {
    ...FIXTURE,
    servers: FIXTURE.servers.map((s) =>
      s.id === 'acme'
        ? {
            ...s,
            compatibility: { status: 'not-required' as const },
            services: { chat: true },
          }
        : s,
    ),
  };
  const base = mockBridge(snapshot);
  const reply = await base.chat('team:eng', { action: 'inbox' });
  if (reply.result.kind !== 'inbox') throw new Error('inbox expected');
  const entries = new Map<string, TeamInbox>([
    [
      'team:eng',
      {
        state: 'ready',
        data: reply.result,
        scope: reply.scope,
        error: '',
        note: '',
        stale: false,
        revision: 1,
        channelRevisions: new Map(),
        blockedChannels: new Set(),
      },
    ],
  ]);
  const listeners = new Set<() => void>();
  const invalidated: string[] = [];
  const inbox = {
    getSnapshot: () => entries,
    subscribe: (listener: () => void) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    invalidate: (store: string) => {
      invalidated.push(store);
    },
    block: () => {},
    handleError: () => false,
    blockChannel: () => {},
    isChannelBlocked: () => false,
  };
  const calls: ChatAction[] = [];
  const bridge = {
    ...base,
    chat: async (store: string, action: ChatAction, view?: string) => {
      calls.push(action);
      const run = () => base.chat(store, action, view);
      return intercept ? intercept(action, run) : run();
    },
  };
  const clock = new Clock();
  const service = new ChatSendService(bridge, inbox, clock);
  service.update(snapshot);
  service.start();
  const channel = reply.result.channels[0].id;
  await service.open('team:eng', channel);
  return {
    service,
    channel,
    calls,
    clock,
    entries,
    invalidated,
    update: () => {
      for (const listener of listeners) listener();
    },
  };
}

test('an access-denied send immediately invalidates the team inbox', async () => {
  const h = await setup(async (action, run) => {
    if (action.action === 'submit-message')
      throw {
        code: 'chat-access-denied',
        message: 'Your role no longer allows chat in this team.',
        fatal: false,
        retryable: false,
        ambiguous: false,
      };
    return run();
  });
  try {
    await h.service.submit('team:eng', h.channel, 'hello').catch(() => {});
    assert.deepEqual(h.invalidated, ['team:eng']);
  } finally {
    h.service.stop();
  }
});

test('a pending send releases its draft while delivery continues outside the conversation', async () => {
  const gate = deferred();
  const h = await setup(async (action, run) => {
    if (action.action === 'submit-message') await gate.promise;
    return run();
  });
  try {
    h.service.setDraft('team:eng', h.channel, 'first');
    const sending = h.service.submit('team:eng', h.channel, 'first');
    assert.equal(h.service.draft('team:eng', h.channel), '');
    assert.equal(h.service.messages('team:eng', h.channel)[0].text, 'first');
    h.service.setDraft('team:eng', h.channel, 'next draft');
    gate.resolve();
    await sending;
    assert.equal(h.service.messages('team:eng', h.channel)[0].phase, 'sent');
    assert.equal(h.service.draft('team:eng', h.channel), 'next draft');
    assert.deepEqual(
      h.calls.map((a) => a.action),
      ['submit-message'],
    );
  } finally {
    gate.resolve();
    h.service.stop();
  }
});

test('one channel intent applies admission backpressure without blocking typing', async () => {
  const gate = deferred();
  const h = await setup(async (action, run) => {
    if (action.action === 'submit-message') await gate.promise;
    return run();
  });
  try {
    const sending = h.service.submit('team:eng', h.channel, 'first');
    h.service.setDraft('team:eng', h.channel, 'second');
    assert.equal(h.service.canSubmit('team:eng', h.channel), false);
    await assert.rejects(h.service.submit('team:eng', h.channel, 'second'));
    assert.equal(h.service.draft('team:eng', h.channel), 'second');
    gate.resolve();
    await sending;
    assert.equal(h.service.canSubmit('team:eng', h.channel), true);
  } finally {
    gate.resolve();
    h.service.stop();
  }
});

test('a lost submit reply is not retried as a new send', async () => {
  let attempts = 0;
  const h = await setup(async (action, run) => {
    if (action.action === 'submit-message') {
      attempts++;
      await run();
      throw {
        code: 'ambiguous',
        message: 'Reply lost',
        ambiguous: true,
        retryable: false,
        fatal: false,
      };
    }
    return run();
  });
  try {
    await h.service.submit('team:eng', h.channel, 'once');
    const message = h.service.messages('team:eng', h.channel)[0];
    await h.service.retry('team:eng', message.id);
    assert.equal(attempts, 1);
    assert.equal(h.service.messages('team:eng', h.channel)[0].phase, 'sent');
  } finally {
    h.service.stop();
  }
});

test('drafts for a team the service has not been handed yet are empty, not a failure', async () => {
  const { service } = await setup();
  try {
    // A location can name a team one render before the provider hands the
    // service the snapshot that holds it; that render reads no drafts.
    assert.doesNotThrow(() => service.drafts('team:not-yet'));
    assert.equal(service.drafts('team:not-yet').size, 0);
    assert.equal(service.drafts('acct:personal').size, 0);
    const drafts = service.drafts('team:eng');
    drafts.set('channel', 'text');
    assert.equal(service.drafts('team:eng').get('channel'), 'text');
  } finally {
    service.stop();
  }
});

test('a send reports its steps to diagnostics without the text', async () => {
  const h = await setup();
  const events: ChatSendTiming[] = [];
  h.service.observe((event) => {
    events.push(event);
  });
  h.service.observe(() => {
    throw new Error('observer failure');
  });
  try {
    await h.service.submit('team:eng', h.channel, 'private text');
    // The mock answers submit-message with a sent operation, so no attempt
    // follows; a prepared operation would add an 'attempted' step.
    assert.deepEqual(
      events.map((event) => event.step),
      ['saved', 'prepared'],
    );
    assert.ok(events.every((event) => event.store === 'team:eng'));
    assert.equal(new Set(events.map((event) => event.message)).size, 1);
    assert.doesNotMatch(JSON.stringify(events), /private text/);
  } finally {
    h.service.stop();
  }
});

const recoveries = (calls: ChatAction[]) =>
  calls.filter((call) => call.action === 'pending').length;

test('an idle team is recovered on the idle interval, not every two seconds', async () => {
  const h = await setup();
  try {
    // The first recovery runs as soon as the service starts, because only
    // the agent's pending list can say what it is holding.
    await h.clock.advance(20_000);
    assert.equal(recoveries(h.calls), 1);
    h.calls.length = 0;
    await h.clock.advance(120_000);
    const runs = recoveries(h.calls);
    // Two seconds apart, as the tick used to run them, this window held
    // sixty rounds of Chat/Pending and Chat/CleanupPending per team.
    assert.ok(runs >= 3 && runs <= 5, `${runs} recoveries in two minutes`);
    assert.equal(
      h.calls.filter((call) => call.action === 'cleanup-pending').length,
      runs,
    );
    assert.deepEqual(h.service.operations('team:eng'), []);
  } finally {
    h.service.stop();
  }
});

test('a send brings the idle tick forward, and the tick goes quiet again after it', async () => {
  const gate = deferred();
  const h = await setup(async (action, run) => {
    if (action.action === 'submit-message') await gate.promise;
    return run();
  });
  try {
    await h.clock.advance(20_000);
    h.calls.length = 0;
    const sending = h.service.submit('team:eng', h.channel, 'hello');
    await settle();
    // A send is work in hand: the tick returns to its active cadence rather
    // than waiting out the idle interval, which had another eleven seconds
    // to run. Its recovery is admitted behind the send itself.
    await h.clock.advance(3_000);
    gate.resolve();
    await sending;
    await settle();
    assert.ok(recoveries(h.calls) >= 1, 'the tick did not come forward');
    assert.equal(h.service.messages('team:eng', h.channel)[0].phase, 'sent');
    await h.clock.advance(20_000);
    h.calls.length = 0;
    await h.clock.advance(120_000);
    const runs = recoveries(h.calls);
    assert.ok(runs <= 5, `${runs} recoveries after the send settled`);
  } finally {
    gate.resolve();
    h.service.stop();
  }
});

test('a failed recovery keeps its own backoff rather than the idle interval', async () => {
  let fail = true;
  const h = await setup(async (action, run) => {
    if (fail && action.action === 'pending')
      throw {
        code: 'unavailable',
        message: 'Temporarily unavailable',
        fatal: false,
        ambiguous: false,
        retryable: true,
      };
    return run();
  });
  try {
    await h.clock.advance(5_000);
    h.calls.length = 0;
    // A team whose recovery failed has still never been recovered, and the
    // failure gate, not the idle interval, says when to try again.
    await h.clock.advance(30_000);
    const attempts = recoveries(h.calls);
    assert.ok(attempts >= 4, `${attempts} retries in thirty seconds`);
    fail = false;
    await h.clock.advance(20_000);
    h.calls.length = 0;
    await h.clock.advance(120_000);
    const runs = recoveries(h.calls);
    assert.ok(runs <= 5, `${runs} recoveries after recovery succeeded`);
  } finally {
    h.service.stop();
  }
});
