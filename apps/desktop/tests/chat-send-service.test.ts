import assert from 'node:assert/strict';
import test from 'node:test';
import { ChatSendService } from '../src/chat/send-service';
import { FIXTURE } from '../src/fixture';
import { mockBridge } from '../src/mock-bridge';
import type { ChatAction, ChatReply } from '../src/chat-contract';
import type { TeamInbox } from '../src/chat/inbox-service';

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  return { promise, resolve };
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
        stale: false,
        revision: 1,
        channelRevisions: new Map(),
        blockedChannels: new Set(),
      },
    ],
  ]);
  const listeners = new Set<() => void>();
  const inbox = {
    getSnapshot: () => entries,
    subscribe: (listener: () => void) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    invalidate: () => {},
    block: () => {},
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
  const service = new ChatSendService(bridge, inbox);
  service.update(snapshot);
  service.start();
  const channel = reply.result.channels[0].id;
  await service.open('team:eng', channel);
  return {
    service,
    channel,
    calls,
    entries,
    update: () => {
      for (const listener of listeners) listener();
    },
  };
}

test('a pending send releases its draft while delivery continues outside the conversation', async () => {
  const gate = deferred();
  const h = await setup(async (action, run) => {
    if (action.action === 'attempt') await gate.promise;
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
    assert.equal(h.calls.filter((a) => a.action === 'attempt').length, 1);
  } finally {
    gate.resolve();
    h.service.stop();
  }
});

test('one channel intent applies admission backpressure without blocking typing', async () => {
  const gate = deferred();
  const h = await setup(async (action, run) => {
    if (action.action === 'prepare-message') await gate.promise;
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

test('uncertain delivery is not retried as a new send', async () => {
  let attempts = 0;
  const h = await setup(async (action, run) => {
    if (action.action === 'attempt') {
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
