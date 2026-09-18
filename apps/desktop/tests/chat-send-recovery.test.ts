import assert from 'node:assert/strict';
import test from 'node:test';
import {
  enqueueProfileWork,
  normalizeCommandError,
  type Bridge,
} from '../src/bridge';
import type {
  ChatAction,
  ChatOperation,
  ChatReply,
} from '../src/chat-contract';
import type { ChatClock, TeamInbox } from '../src/chat/inbox-service';
import { ChatSendService } from '../src/chat/send-service';
import { chatIntentPersistence } from '../src/chat/intent';
import { FIXTURE } from '../src/fixture';
import { mockBridge } from '../src/mock-bridge';

const STORE = 'team:eng';
const unavailable = {
  code: 'unavailable',
  message: 'Temporarily unavailable',
  fatal: false,
  ambiguous: false,
  retryable: true,
};
const lostReply = {
  ...unavailable,
  code: 'ambiguous',
  message: 'Reply lost',
  ambiguous: true,
  retryable: false,
};
const settle = () => new Promise<void>((resolve) => setImmediate(resolve));

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

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

type LocalAction = Parameters<Bridge['chatLocal']>[0];
type LocalReply = Awaited<ReturnType<Bridge['chatLocal']>>;
type Hooks = {
  chat?: (
    action: ChatAction,
    run: (action?: ChatAction) => Promise<ChatReply>,
  ) => Promise<ChatReply>;
  local?: (
    action: LocalAction,
    run: () => Promise<LocalReply>,
  ) => Promise<LocalReply>;
};

async function setup(hooks: Hooks = {}) {
  const snapshot = {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server) =>
      server.id === 'acme'
        ? {
            ...server,
            compatibility: { status: 'not-required' as const },
            services: { chat: true },
          }
        : server,
    ),
  };
  const base = mockBridge(snapshot);
  const reply = await base.chat(STORE, { action: 'inbox' });
  if (reply.result.kind !== 'inbox') throw new Error('inbox expected');
  const ready: TeamInbox = {
    state: 'ready',
    data: reply.result,
    scope: reply.scope,
    error: '',
    stale: false,
    revision: 1,
    channelRevisions: new Map(),
    blockedChannels: new Set(),
  };
  const entries = new Map([[STORE, ready]]);
  const listeners = new Set<() => void>();
  const calls: ChatAction[] = [];
  const localCalls: LocalAction[] = [];
  const invalidations: string[] = [];
  const updateInbox = (entry: TeamInbox) => {
    entries.set(STORE, entry);
    for (const listener of listeners) listener();
  };
  const inbox = {
    getSnapshot: () => entries,
    subscribe: (listener: () => void) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    invalidate: (store: string) => {
      invalidations.push(store);
    },
    block: (_store: string, error: string) => {
      updateInbox({ ...entries.get(STORE)!, state: 'blocked', error });
    },
    blockChannel: (_store: string, channel: string) => {
      updateInbox({
        ...entries.get(STORE)!,
        blockedChannels: new Set([channel]),
      });
    },
    handleError: (store: string, cause: unknown, channel?: string) => {
      const error = normalizeCommandError(cause);
      if (error.code === 'chat-channel-integrity' && channel)
        inbox.blockChannel(store, channel);
      else if (error.code === 'chat-integrity')
        inbox.block(store, error.message);
      else if (error.code === 'chat-access-denied') inbox.invalidate(store);
      else return false;
      return true;
    },
    isChannelBlocked: (_store: string, channel: string) =>
      entries.get(STORE)!.blockedChannels.has(channel),
  };
  const bridge: Bridge = {
    ...base,
    chat: async (store, action, view) => {
      calls.push(structuredClone(action));
      const run = async (input = action) =>
        structuredClone(await base.chat(store, input, view));
      return hooks.chat ? hooks.chat(action, run) : run();
    },
    chatLocal: async (action) => {
      localCalls.push(structuredClone(action));
      const run = () => base.chatLocal(action);
      return hooks.local ? hooks.local(action, run) : run();
    },
  };
  const clock = new Clock();
  const service = new ChatSendService(bridge, inbox, clock);
  service.update(snapshot);
  service.start();
  const channel = reply.result.channels[0].id;
  await service.open(STORE, channel);
  return {
    service,
    base,
    bridge,
    snapshot,
    ready,
    channel,
    calls,
    localCalls,
    clock,
    updateInbox,
    invalidations,
    scope: reply.scope,
  };
}

function count(calls: ChatAction[], action: ChatAction['action']) {
  return calls.filter((call) => call.action === action).length;
}

test('fresh message persists its intent before a single submit request', async () => {
  const order: string[] = [];
  const h = await setup({
    local: async (action, run) => {
      const reply = await run();
      order.push(action.action);
      return reply;
    },
    chat: async (action, run) => {
      order.push(action.action);
      return run();
    },
  });
  try {
    await h.service.submit(STORE, h.channel, 'one request');
    assert.deepEqual(
      h.calls.map((call) => call.action),
      ['submit-message'],
    );
    assert.ok(order.indexOf('save-intent') < order.indexOf('submit-message'));
    assert.equal(h.service.messages(STORE, h.channel)[0].phase, 'sent');
    assert.equal(h.service.operations(STORE)[0].text, 'one request');
  } finally {
    h.service.stop();
  }
});

for (const state of ['prepared', 'uncertain'] as const) {
  test(`submit returning retained ${state} only attempts verified prepared work`, async () => {
    const h = await setup({
      chat: async (action, run) => {
        if (action.action === 'submit-message') {
          const reply = await run({ ...action, action: 'prepare-message' });
          const pending = await h.base.chat(STORE, { action: 'pending' });
          if (pending.result.kind !== 'pending')
            throw new Error('pending expected');
          pending.result.operations[0].state = state;
          if (reply.result.kind === 'operation')
            reply.result.operation.state = state;
          return reply;
        }
        return run();
      },
    });
    try {
      await h.service.submit(STORE, h.channel, 'retained body');
      const message = h.service.messages(STORE, h.channel)[0];
      assert.equal(
        message.phase,
        state === 'prepared' ? 'sent' : 'unconfirmed',
      );
      await h.service.retry(STORE, message.id);
      await h.clock.advance(7000);
      assert.equal(count(h.calls, 'submit-message'), 1);
      assert.equal(count(h.calls, 'attempt'), state === 'prepared' ? 1 : 0);
      if (state === 'uncertain') assert.ok(count(h.calls, 'reconcile') > 0);
    } finally {
      h.service.stop();
    }
  });
}

test('lost legacy recovery preparation replies retain the original submit identity', async () => {
  let losePreparation = true;
  const h = await setup({
    chat: async (action, run) => {
      if (action.action === 'submit-message') throw lostReply;
      const reply = await run();
      if (action.action === 'prepare-message' && losePreparation) {
        losePreparation = false;
        throw lostReply;
      }
      return reply;
    },
  });
  try {
    await h.service.submit(STORE, h.channel, 'original recovery body');
    const original = h.service.messages(STORE, h.channel)[0];
    await h.clock.advance(10000);
    const preparations = h.calls.filter(
      (call) => call.action === 'prepare-message',
    );
    assert.equal(preparations.length, 2);
    assert.ok(
      preparations.every(
        (call) =>
          call.submission === original.submission &&
          call.channel === h.channel &&
          call.text === original.text,
      ),
    );
    assert.equal(count(h.calls, 'submit-message'), 1);
    assert.equal(count(h.calls, 'attempt'), 1);
    assert.equal(h.service.messages(STORE, h.channel)[0].phase, 'sent');
  } finally {
    h.service.stop();
  }
});

test('a lost submit reply after its uncertain checkpoint recovers without replay', async () => {
  const h = await setup({
    chat: async (action, run) => {
      if (action.action === 'submit-message') {
        await run({ ...action, action: 'prepare-message' });
        const pending = await h.base.chat(STORE, { action: 'pending' });
        if (pending.result.kind !== 'pending')
          throw new Error('pending expected');
        pending.result.operations[0].state = 'uncertain';
        throw lostReply;
      }
      return run();
    },
  });
  try {
    await h.service.submit(STORE, h.channel, 'checkpointed body');
    const original = h.service.messages(STORE, h.channel)[0];
    assert.equal(original.intentPending, true);
    await h.clock.advance(7000);
    const recovered = h.service.messages(STORE, h.channel)[0];
    assert.equal(recovered.id, original.id);
    assert.equal(recovered.operation?.state, 'uncertain');
    assert.equal(recovered.phase, 'unconfirmed');
    assert.equal(recovered.text, 'checkpointed body');
    await h.service.retry(STORE, recovered.id);
    assert.equal(count(h.calls, 'submit-message'), 1);
    assert.equal(count(h.calls, 'prepare-message'), 1);
    assert.equal(count(h.calls, 'attempt'), 0);
    assert.ok(count(h.calls, 'reconcile') > 0);
    await assert.rejects(h.service.restoreDraft(STORE, recovered.id));
  } finally {
    h.service.stop();
  }
});

for (const committed of [false, true]) {
  test(`restored intent uses legacy prepare only after lost submit committed=${committed}`, async () => {
    const h = await setup({
      chat: async (action, run) => {
        if (action.action === 'submit-message') {
          if (committed) await run();
          throw new Error('Reply lost');
        }
        return run();
      },
    });
    try {
      await h.service.submit(STORE, h.channel, 'protected body');
      const original = await chatIntentPersistence(
        h.bridge,
        STORE,
        h.scope,
        h.channel,
      ).load();
      assert.ok(original);
      assert.equal(
        h.service.messages(STORE, h.channel)[0].phase,
        'unconfirmed',
      );
      assert.equal(h.service.canSubmit(STORE, h.channel), false);
      h.service.stop();
      h.updateInbox(h.ready);
      h.service.start();
      await h.service.open(STORE, h.channel);
      await settle();
      const restored = h.service.messages(STORE, h.channel)[0];
      assert.equal(restored.id, original.submission);
      assert.equal(restored.ambiguousPreparation, true);
      assert.equal(restored.phase, 'sent');
      assert.equal(count(h.calls, 'submit-message'), 1);
      assert.deepEqual(
        h.calls.find((call) => call.action === 'prepare-message'),
        {
          action: 'prepare-message',
          submission: original.submission,
          channel: h.channel,
          text: 'protected body',
        },
      );
      assert.equal(count(h.calls, 'attempt'), committed ? 0 : 1);
    } finally {
      h.service.stop();
    }
  });
}

for (const boundary of [
  'generation',
  'locked',
  'read-only',
  'stopped',
] as const) {
  test(`queued submit rechecks ${boundary} eligibility before dispatch`, async () => {
    const h = await setup();
    const gate = deferred();
    const hold = enqueueProfileWork(h.bridge, 'acme', () => gate.promise);
    let sending: Promise<void> | undefined;
    try {
      await settle();
      sending = h.service.submit(STORE, h.channel, 'queued body');
      await settle();
      if (boundary === 'generation')
        h.service.update(h.snapshot, new Map([['acme', 1]]));
      else if (boundary === 'locked')
        h.service.update({
          ...h.snapshot,
          agent: { state: 'bootstrap', step: 'locked' },
        });
      else if (boundary === 'read-only')
        h.updateInbox({
          ...h.ready,
          data: {
            ...h.ready.data!,
            channels: h.ready.data!.channels.map((c) => ({
              ...c,
              writable: false,
            })),
          },
        });
      else h.service.stop();
      gate.resolve();
      await hold;
      await sending;
      assert.equal(count(h.calls, 'submit-message'), 0);
      assert.equal(count(h.calls, 'attempt'), 0);
      assert.equal(
        h.localCalls.some((call) => call.action === 'clear-intent'),
        false,
      );
      if (boundary !== 'stopped') {
        const message = h.service.messages(STORE, h.channel)[0];
        assert.equal(message.intentPending, true);
        assert.equal(message.phase, 'unconfirmed');
      }
    } finally {
      gate.resolve();
      await hold;
      await sending;
      h.service.stop();
    }
  });
}

test('failed local intent deletion does not hide known delivery or admit another same-slot intent', async () => {
  let failClear = true;
  const h = await setup({
    local: async (action, run) => {
      if (action.action === 'clear-intent' && failClear) throw unavailable;
      return run();
    },
  });
  try {
    await h.service.submit(STORE, h.channel, 'delivered');
    await settle();
    const sent = h.service.messages(STORE, h.channel)[0];
    assert.equal(sent.phase, 'sent');
    assert.equal(sent.operation?.state, 'confirmed');
    assert.equal(sent.intentPending, true);
    assert.notEqual(sent.cleanupError, '');
    h.service.setDraft(STORE, h.channel, 'next draft');
    assert.equal(h.service.canSubmit(STORE, h.channel), false);
    await assert.rejects(h.service.submit(STORE, h.channel, 'next draft'));
    assert.equal(h.service.draft(STORE, h.channel), 'next draft');
    failClear = false;
    await h.clock.advance(6000);
    assert.equal(h.service.messages(STORE, h.channel)[0].intentPending, false);
    assert.equal(h.service.canSubmit(STORE, h.channel), true);
    assert.equal(count(h.calls, 'submit-message'), 1);
  } finally {
    h.service.stop();
  }
});

test('delayed local intent deletion does not hold delivery completion and keeps same-slot backpressure', async () => {
  const clearing = deferred();
  const h = await setup({
    local: async (action, run) => {
      if (action.action === 'clear-intent') await clearing.promise;
      return run();
    },
  });
  let sending: Promise<void> | undefined;
  try {
    let complete = false;
    sending = h.service.submit(STORE, h.channel, 'delivered first').then(() => {
      complete = true;
    });
    await settle();
    assert.equal(h.service.messages(STORE, h.channel)[0].phase, 'sent');
    assert.equal(complete, true);
    assert.equal(h.service.canSubmit(STORE, h.channel), false);
    await h.clock.advance(5000);
    assert.equal(
      h.localCalls.filter((call) => call.action === 'clear-intent').length,
      1,
    );
    clearing.resolve();
    await settle();
    assert.equal(h.service.canSubmit(STORE, h.channel), true);
  } finally {
    clearing.resolve();
    await sending;
    h.service.stop();
  }
});

test('repeated local intent cleanup failures use increasing automatic retry backoff', async () => {
  const times: number[] = [];
  const h = await setup({
    local: async (action, run) => {
      if (action.action === 'clear-intent') {
        times.push(h.clock.now());
        throw unavailable;
      }
      return run();
    },
  });
  try {
    await h.service.submit(STORE, h.channel, 'cleanup retry');
    await h.clock.advance(30000);
    assert.ok(times.length >= 3, 'failed local cleanup must be retried');
    assert.ok(
      times.length <= 8,
      `expected backoff, observed ${times.length} clear attempts in 30 seconds`,
    );
    assert.equal(h.service.messages(STORE, h.channel)[0].phase, 'sent');
    assert.equal(count(h.calls, 'submit-message'), 1);
  } finally {
    h.service.stop();
  }
});

for (const committed of [false, true]) {
  for (const error of [
    new Error('Reply lost'),
    lostReply,
    { ...unavailable, code: 'chat-limit' },
    { ...unavailable, code: 'access-denied' },
  ]) {
    test(`lost submit reply committed=${committed} error=${'code' in error ? error.code : 'Error'} recovers the exact submission`, async () => {
      const h = await setup({
        chat: async (action, run) => {
          if (action.action === 'submit-message') {
            if (committed) await run();
            throw error;
          }
          return run();
        },
      });
      try {
        await h.service.submit(STORE, h.channel, 'original');
        const message = h.service.messages(STORE, h.channel)[0];
        assert.equal(message.phase, 'unconfirmed');
        assert.equal(message.ambiguousPreparation, true);
        assert.equal(message.intentPending, true);
        assert.equal(h.service.canSubmit(STORE, h.channel), false);
        await assert.rejects(h.service.restoreDraft(STORE, message.id));
        assert.equal(
          h.localCalls.some((call) => call.action === 'clear-intent'),
          false,
        );
        h.service.setDraft(STORE, h.channel, 'new draft');
        await h.clock.advance(7000);
        const preparations = h.calls.filter(
          (call) => call.action === 'prepare-message',
        );
        assert.equal(preparations.length, 1);
        assert.deepEqual(preparations[0], {
          action: 'prepare-message',
          submission: message.submission,
          channel: h.channel,
          text: 'original',
        });
        assert.equal(count(h.calls, 'submit-message'), 1);
        assert.equal(count(h.calls, 'attempt'), committed ? 0 : 1);
        assert.equal(h.service.messages(STORE, h.channel)[0].phase, 'sent');
        assert.equal(h.service.messages(STORE, h.channel)[0].id, message.id);
        assert.equal(h.service.draft(STORE, h.channel), 'new draft');
        const history = await h.base.chat(STORE, {
          action: 'history',
          channel: h.channel,
          before: null,
        });
        assert.equal(history.result.kind, 'history');
        if (history.result.kind === 'history')
          assert.equal(
            history.result.messages.filter(
              (m) => m.content.kind === 'text' && m.content.text === 'original',
            ).length,
            1,
          );
      } finally {
        h.service.stop();
      }
    });
  }
}

for (const mode of ['unknown', 'uncertain'] as const) {
  test(`${mode} attempt failure never causes an automatic resend`, async () => {
    const h = await setup({
      chat: async (action, run) => {
        if (action.action === 'submit-message')
          return run({ ...action, action: 'prepare-message' });
        if (action.action === 'attempt') throw lostReply;
        if (action.action === 'status' && mode === 'unknown') throw unavailable;
        const reply = await run();
        if (mode === 'uncertain') {
          if (
            reply.result.kind === 'operation' &&
            ['status', 'reconcile'].includes(action.action)
          )
            reply.result.operation.state = 'uncertain';
          if (reply.result.kind === 'pending')
            reply.result.operations = reply.result.operations.map((op) => ({
              ...op,
              state: 'uncertain',
            }));
        }
        return reply;
      },
    });
    try {
      await h.service.submit(STORE, h.channel, 'only once');
      await h.clock.advance(30000);
      assert.equal(count(h.calls, 'submit-message'), 1);
      assert.equal(count(h.calls, 'prepare-message'), 0);
      assert.equal(count(h.calls, 'attempt'), 1);
      assert.notEqual(h.service.messages(STORE, h.channel)[0].phase, 'sent');
      if (mode === 'uncertain') assert.ok(count(h.calls, 'reconcile') > 0);
    } finally {
      h.service.stop();
    }
  });
}

test('delayed local status recovery does not hold the primary submit completion', async () => {
  const status = deferred();
  let statusStarted = false;
  const h = await setup({
    chat: async (action, run) => {
      if (action.action === 'submit-message')
        return run({ ...action, action: 'prepare-message' });
      if (action.action === 'attempt') throw lostReply;
      if (action.action === 'status') {
        statusStarted = true;
        await status.promise;
      }
      return run();
    },
  });
  let sending: Promise<void> | undefined;
  try {
    let complete = false;
    sending = h.service.submit(STORE, h.channel, 'unconfirmed').then(() => {
      complete = true;
    });
    await settle();
    assert.equal(statusStarted, true);
    assert.equal(h.service.messages(STORE, h.channel)[0].phase, 'unconfirmed');
    assert.equal(
      complete,
      true,
      'submit must settle before the delayed status lookup settles',
    );
  } finally {
    status.resolve();
    await sending;
    h.service.stop();
  }
});

test('observed history deduplicates delivery and leaves previously returned operation state unchanged', async () => {
  const h = await setup();
  try {
    await h.service.submit(STORE, h.channel, 'history text');
    const oldOperations = h.service.operations(STORE);
    const before = structuredClone(oldOperations);
    const sent = h.service.messages(STORE, h.channel)[0];
    const reply = await h.base.chat(STORE, {
      action: 'history',
      channel: h.channel,
      before: null,
    });
    if (reply.result.kind !== 'history') throw new Error('history expected');
    h.service.observeHistory(STORE, h.channel, reply.result.messages);
    h.service.observeHistory(STORE, h.channel, reply.result.messages);
    assert.deepEqual(oldOperations, before);
    assert.equal(
      h.service
        .messages(STORE, h.channel)
        .filter((message) => !message.observed).length,
      0,
    );
    assert.equal(
      h.service.operations(STORE).filter((op) => op.id === sent.operation!.id)
        .length,
      1,
    );
    assert.equal(h.service.messageKey(STORE, sent.operation!.id), sent.id);
    assert.equal(h.service.operations(STORE)[0].text, undefined);
    assert.equal(count(h.calls, 'submit-message'), 1);
  } finally {
    h.service.stop();
  }
});

test('history observation does not mutate a previously returned outgoing message snapshot', async () => {
  const h = await setup();
  try {
    await h.service.submit(STORE, h.channel, 'snapshot text');
    const oldMessages = h.service.messages(STORE, h.channel);
    const before = structuredClone(oldMessages);
    const reply = await h.base.chat(STORE, {
      action: 'history',
      channel: h.channel,
      before: null,
    });
    if (reply.result.kind !== 'history') throw new Error('history expected');
    h.service.observeHistory(STORE, h.channel, reply.result.messages);
    assert.deepEqual(oldMessages, before);
  } finally {
    h.service.stop();
  }
});

for (const stage of ['saving', 'preparing'] as const) {
  for (const boundary of ['stop', 'lock'] as const) {
    test(`${boundary} during ${stage} prevents late replies from preparing, attempting, clearing, or invalidating`, async () => {
      const gate = deferred();
      let started = false;
      const h = await setup({
        local: async (action, run) => {
          if (stage === 'saving' && action.action === 'save-intent') {
            started = true;
            await gate.promise;
          }
          return run();
        },
        chat: async (action, run) => {
          const reply = await run();
          if (stage === 'preparing' && action.action === 'submit-message') {
            started = true;
            await gate.promise;
          }
          return reply;
        },
      });
      let sending: Promise<void> | undefined;
      try {
        sending = h.service.submit(STORE, h.channel, 'private');
        await settle();
        assert.equal(started, true);
        if (boundary === 'stop') h.service.stop();
        else
          h.service.update({
            ...h.snapshot,
            agent: { state: 'bootstrap', step: 'locked' },
          });
        const prepareCount = count(h.calls, 'submit-message');
        const clearCount = h.localCalls.filter(
          (call) => call.action === 'clear-intent',
        ).length;
        const invalidationCount = h.invalidations.length;
        gate.resolve();
        await sending;
        await h.clock.advance(5000);
        assert.equal(count(h.calls, 'submit-message'), prepareCount);
        assert.equal(count(h.calls, 'attempt'), 0);
        assert.equal(
          h.localCalls.filter((call) => call.action === 'clear-intent').length,
          clearCount,
        );
        assert.equal(h.invalidations.length, invalidationCount);
        if (boundary === 'stop')
          assert.deepEqual(h.service.messages(STORE, h.channel), []);
        else
          assert.notEqual(
            h.service.messages(STORE, h.channel)[0]?.phase,
            'sent',
          );
      } finally {
        gate.resolve();
        await sending;
        h.service.stop();
      }
    });
  }
}

test('progressive temporary scope loss preserves draft and submitted identity, actual replacement clears them', async () => {
  const h = await setup();
  try {
    await h.service.submit(STORE, h.channel, 'submitted');
    h.service.setDraft(STORE, h.channel, 'draft');
    const id = h.service.messages(STORE, h.channel)[0].id;
    h.updateInbox({
      ...h.ready,
      state: 'unavailable',
      scope: undefined,
      data: undefined,
      stale: true,
    });
    h.service.update({
      ...h.snapshot,
      stores: h.snapshot.stores.filter((store) => store.id !== STORE),
      profileInventory: h.snapshot.profileInventory.map((profile) => ({
        ...profile,
        teams: 'unavailable' as const,
      })),
    });
    assert.equal(h.service.draft(STORE, h.channel), 'draft');
    assert.equal(h.service.messages(STORE, h.channel)[0].id, id);
    h.service.update(h.snapshot);
    h.updateInbox(h.ready);
    assert.equal(h.service.draft(STORE, h.channel), 'draft');
    assert.equal(h.service.messages(STORE, h.channel)[0].id, id);
    h.updateInbox({
      ...h.ready,
      scope: { ...h.scope, actor: '01' + 'cd'.repeat(32) },
    });
    assert.equal(h.service.draft(STORE, h.channel), '');
    assert.deepEqual(h.service.messages(STORE, h.channel), []);
    assert.deepEqual(h.service.operations(STORE), []);
  } finally {
    h.service.stop();
  }
});

function operations(
  channel: string,
  state: 'uncertain' | 'confirmed',
  total = 12,
): ChatOperation[] {
  return Array.from({ length: total }, (_, index) => ({
    id: (index + 1000).toString(16).padStart(32, '0'),
    channel,
    kind: 'send-message',
    state,
    receipt:
      state === 'confirmed'
        ? { kind: 'message-sent', sequence: String(index + 1) }
        : null,
    rejection_code: null,
  }));
}

test('automatic reconciliation fairly reaches more than eight operations with per-operation backoff', async () => {
  let rows: ChatOperation[] = [];
  const attempts = new Map<string, number[]>();
  const h = await setup({
    chat: async (action, run) => {
      if (action.action === 'operation-body')
        return {
          scope: h.scope,
          result: {
            kind: 'operation-body',
            operation: action.operation,
            channel: action.channel,
            text: null,
          },
        };
      if (action.action === 'reconcile') {
        attempts.set(action.operation, [
          ...(attempts.get(action.operation) ?? []),
          h.clock.now(),
        ]);
        throw unavailable;
      }
      const reply = await run();
      if (action.action === 'pending')
        return {
          ...reply,
          result: { kind: 'pending', operations: structuredClone(rows) },
        };
      return reply;
    },
  });
  rows = operations(h.channel, 'uncertain');
  try {
    await h.clock.advance(20000);
    assert.equal(
      attempts.size,
      rows.length,
      'every uncertain operation must be checked despite a failing prefix',
    );
    assert.equal(count(h.calls, 'attempt'), 0);
    for (const times of attempts.values()) {
      assert.ok(times.length >= 2);
      assert.ok(
        times.length < 9,
        'failed reconciliation should back off instead of polling every cycle',
      );
      for (let index = 1; index < times.length; index++)
        assert.ok(times[index] - times[index - 1] >= 2000);
    }
  } finally {
    h.service.stop();
  }
});

test('automatic terminal cleanup fairly reaches entries beyond a repeatedly failing first eight', async () => {
  let rows: ChatOperation[] = [];
  const attempts = new Map<string, number[]>();
  const h = await setup({
    chat: async (action, run) => {
      if (action.action === 'operation-body')
        return {
          scope: h.scope,
          result: {
            kind: 'operation-body',
            operation: action.operation,
            channel: action.channel,
            text: null,
          },
        };
      if (action.action === 'finalize') {
        attempts.set(action.operation, [
          ...(attempts.get(action.operation) ?? []),
          h.clock.now(),
        ]);
        throw unavailable;
      }
      const reply = await run();
      if (action.action === 'cleanup-pending')
        return {
          ...reply,
          result: {
            kind: 'cleanup-pending',
            operations: structuredClone(rows),
          },
        };
      return reply;
    },
  });
  rows = operations(h.channel, 'confirmed');
  try {
    await h.clock.advance(20000);
    assert.equal(
      attempts.size,
      rows.length,
      'failing first eight cleanup entries must not starve later entries',
    );
    for (const times of attempts.values()) assert.ok(times.length < 9);
  } finally {
    h.service.stop();
  }
});

test('a large failing cleanup backlog does not starve automatic uncertain-delivery reconciliation', async () => {
  let cleanup: ChatOperation[] = [];
  let pending: ChatOperation[] = [];
  const reconciled = new Set<string>();
  const h = await setup({
    chat: async (action, run) => {
      if (action.action === 'finalize') throw unavailable;
      if (action.action === 'operation-body')
        return {
          scope: h.scope,
          result: {
            kind: 'operation-body',
            operation: action.operation,
            channel: action.channel,
            text: null,
          },
        };
      if (action.action === 'reconcile') {
        reconciled.add(action.operation);
        return {
          scope: h.scope,
          result: {
            kind: 'operation',
            operation: structuredClone(
              pending.find((op) => op.id === action.operation)!,
            ),
          },
        };
      }
      const reply = await run();
      if (action.action === 'pending')
        return {
          ...reply,
          result: { kind: 'pending', operations: structuredClone(pending) },
        };
      if (action.action === 'cleanup-pending')
        return {
          ...reply,
          result: {
            kind: 'cleanup-pending',
            operations: structuredClone(cleanup),
          },
        };
      return reply;
    },
  });
  cleanup = operations(h.channel, 'confirmed');
  pending = operations(h.channel, 'uncertain', 1).map((op) => ({
    ...op,
    id: 'f'.repeat(32),
  }));
  try {
    await h.clock.advance(20000);
    assert.equal(
      reconciled.size,
      1,
      'uncertain delivery must receive reconciliation while terminal cleanup remains unavailable',
    );
    assert.equal(count(h.calls, 'attempt'), 0);
  } finally {
    h.service.stop();
  }
});

test('verified history arriving before a submit reply is merged without another delivery', async () => {
  const gate = deferred();
  let reached = false;
  const h = await setup({
    chat: async (action, run) => {
      const reply = await run();
      if (
        action.action === 'submit-message' &&
        reply.result.kind === 'operation'
      ) {
        const history = await h.base.chat(STORE, {
          action: 'history',
          channel: action.channel,
          before: null,
        });
        if (history.result.kind !== 'history')
          throw new Error('history expected');
        h.service.observeHistory(
          STORE,
          action.channel,
          history.result.messages,
        );
        reached = true;
        await gate.promise;
      }
      return reply;
    },
  });
  try {
    const sending = h.service.submit(STORE, h.channel, 'already observed');
    await settle();
    assert.equal(reached, true);
    gate.resolve();
    await sending;
    assert.equal(count(h.calls, 'attempt'), 0);
    assert.equal(
      h.service.messages(STORE, h.channel).filter((m) => !m.observed).length,
      0,
    );
    assert.equal(h.service.messages(STORE, h.channel)[0].phase, 'sent');
  } finally {
    gate.resolve();
    h.service.stop();
  }
});

test('a changed identity is explicitly quarantined until the unlocked owner is replaced', async () => {
  const h = await setup();
  try {
    h.service.setDraft(STORE, h.channel, 'private draft');
    h.updateInbox({
      ...h.ready,
      scope: { ...h.scope, actor: '01' + 'cd'.repeat(32) },
    });
    assert.equal(h.service.draft(STORE, h.channel), '');
    assert.equal(h.service.canSubmit(STORE, h.channel), false);
    await h.service.open(STORE, h.channel);
    assert.equal(h.service.loadError(STORE, h.channel), '');
    assert.throws(() => h.service.setDraft(STORE, h.channel, 'new text'));
    assert.equal(h.service.messages(STORE, h.channel).length, 0);
  } finally {
    h.service.stop();
  }
});

test('terminal cleanup retries after failure and stops after durable success', async () => {
  let finalized = false;
  let tries = 0;
  const h = await setup({
    chat: async (action, run) => {
      if (action.action === 'operation-body')
        return {
          scope: h.scope,
          result: {
            kind: 'operation-body',
            operation: action.operation,
            channel: action.channel,
            text: null,
          },
        };
      if (action.action === 'finalize') {
        if (++tries === 1) throw unavailable;
        finalized = true;
        return {
          scope: h.scope,
          result: { kind: 'operation', operation: structuredClone(row) },
        };
      }
      const reply = await run();
      if (action.action === 'cleanup-pending')
        return {
          ...reply,
          result: {
            kind: 'cleanup-pending',
            operations: row && !finalized ? [structuredClone(row)] : [],
          },
        };
      return reply;
    },
  });
  const row = operations(h.channel, 'confirmed', 1)[0];
  try {
    await h.clock.advance(1000);
    assert.equal(tries, 1);
    assert.notEqual(h.service.cleanupError(STORE), '');
    await h.clock.advance(10000);
    assert.equal(tries, 2);
    assert.equal(h.service.cleanupError(STORE), '');
    assert.equal(
      h.service.operations(STORE).find((op) => op.id === row.id)?.state,
      'confirmed',
    );
    h.service.observeHistory(STORE, h.channel, [
      {
        id: row.id,
        sequence: '1',
        sender: h.scope.actor,
        send_time: '1',
        insert_time: '1',
        content: { kind: 'text', text: 'confirmed' },
      },
    ]);
    assert.equal(
      h.service.operations(STORE).find((op) => op.id === row.id)?.observed,
      true,
    );
  } finally {
    h.service.stop();
  }
});
