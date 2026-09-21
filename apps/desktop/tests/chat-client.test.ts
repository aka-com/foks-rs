import assert from 'node:assert/strict';
import test from 'node:test';
import { chatClient } from '../src/chat/client';
import type { Bridge } from '../src/bridge';
import type { ChatReply } from '../src/chat-contract';
const reply: ChatReply = {
  scope: {
    store: { profile: 'p', account_alias: 'a', team_alias: 't', team_id: '03' },
    host: 'host',
    actor: 'actor',
  },
  result: { kind: 'pending', operations: [] },
};
test('request owner cancels late results and releases its view once', async () => {
  let resolve!: (r: ChatReply) => void;
  let calls = 0;
  const bridge = {
    chat: () =>
      new Promise<ChatReply>((r) => {
        resolve = r;
      }),
    cancelChat: async () => {
      calls++;
    },
  } as unknown as Bridge;
  const client = chatClient(bridge, 'p', 't');
  const request = client.request({ action: 'pending' });
  await new Promise((r) => setImmediate(r));
  client.dispose();
  client.dispose();
  resolve(reply);
  await assert.rejects(request, { code: 'cancelled' });
  assert.equal(calls, 1);
});
test('owner rejects changed actor and action result binding', async () => {
  let next = reply;
  const bridge = {
    chat: async () => next,
    cancelChat: async () => {},
  } as unknown as Bridge;
  const client = chatClient(bridge, 'p', 't');
  await client.run({ action: 'pending' });
  next = { ...reply, scope: { ...reply.scope, actor: 'other' } };
  await assert.rejects(client.request({ action: 'pending' }), {
    code: 'chat-integrity',
  });
  next = reply;
  await assert.rejects(client.request({ action: 'channels' }), {
    code: 'chat-integrity',
  });
  client.dispose();
});

test('poll bypasses queued work and only bounded notification history may request background admission', async () => {
  const { scheduleProfileWork } =
    await import('../src/scheduling/profile-work');
  let release!: () => void;
  const bridge = {
    chat: async () => ({ scope: reply.scope, result: { kind: 'poll' } }),
    cancelChat: async () => {},
  } as unknown as Bridge;
  const hold = scheduleProfileWork(
    bridge,
    'p',
    () =>
      new Promise<void>((r) => {
        release = r;
      }),
  );
  await new Promise((r) => setImmediate(r));
  const client = chatClient(bridge, 'p', 't');
  assert.equal(
    (
      await client.request({
        action: 'poll-inbox',
        since: '0',
        timeout_milliseconds: 1,
      })
    ).result.kind,
    'poll',
  );
  await assert.rejects(
    client.request(
      { action: 'pending' },
      {
        key: 'key',
        owner: {},
        generation: 1,
        signal: new AbortController().signal,
        current: () => true,
        cancel: () => {},
        preemptible: false,
      },
    ),
    { code: 'chat-integrity' },
  );
  await assert.rejects(
    client.request(
      { action: 'history', channel: 'channel', before: null },
      {
        key: 'key',
        owner: {},
        generation: 1,
        signal: new AbortController().signal,
        current: () => true,
        cancel: () => {},
        preemptible: false,
      },
    ),
    { code: 'chat-integrity' },
  );
  release();
  await hold;
  client.dispose();
});

test('chat work runs while other profile work is active, and foreground history overtakes queued chat work', async () => {
  const { scheduleProfileWork } =
    await import('../src/scheduling/profile-work');
  const order: string[] = [];
  let release!: () => void;
  let releaseFirst!: () => void;
  let firstStarted!: () => void;
  const first = new Promise<void>((resolve) => {
    firstStarted = resolve;
  });
  let calls = 0;
  const bridge = {
    chat: async (_store: string, action: { action: string }) => {
      order.push(action.action);
      if (++calls === 1) {
        firstStarted();
        await new Promise<void>((resolve) => {
          releaseFirst = resolve;
        });
      }
      return {
        scope: reply.scope,
        result: {
          kind: action.action === 'sync-inbox' ? 'inbox' : action.action,
        },
      };
    },
    cancelChat: async () => {},
  } as unknown as Bridge;
  const active = scheduleProfileWork(
    bridge,
    'p',
    () =>
      new Promise<void>((resolve) => {
        release = resolve;
      }),
  );
  await new Promise((resolve) => setImmediate(resolve));
  const client = chatClient(bridge, 'p', 't');
  const channels = client.request({ action: 'channels' });
  // The chat lane admits its work although the profile's other lane is busy.
  await first;
  assert.deepEqual(order, ['channels']);
  const inbox = client.request({ action: 'sync-inbox', blocked_channels: [] });
  const recovery = client.request(
    { action: 'pending' },
    undefined,
    undefined,
    'background',
  );
  const history = client.request({
    action: 'history',
    channel: 'a',
    before: null,
  });
  await new Promise((resolve) => setImmediate(resolve));
  // One chat request at a time: the rest wait for the active one to settle.
  assert.deepEqual(order, ['channels']);
  releaseFirst();
  release();
  await Promise.all([active, channels, inbox, recovery, history]);
  assert.equal(order[1], 'history');
  assert.deepEqual(
    new Set(order),
    new Set(['channels', 'history', 'sync-inbox', 'pending']),
  );
  client.dispose();
});

test('queued chat work rechecks access at actual RPC dispatch', async () => {
  const { scheduleProfileWork } =
    await import('../src/scheduling/profile-work');
  let release!: () => void;
  let calls = 0;
  let allowed = true;
  const bridge = {
    chat: async () => {
      calls++;
      return reply;
    },
    cancelChat: async () => {},
  } as unknown as Bridge;
  const hold = scheduleProfileWork(
    bridge,
    'p',
    () => new Promise<void>((resolve) => (release = resolve)),
  );
  await new Promise((resolve) => setImmediate(resolve));
  const client = chatClient(bridge, 'p', 't');
  const pending = client.request({ action: 'pending' }, undefined, () => {
    if (!allowed) throw { code: 'check-in-expired' };
  });
  allowed = false;
  release();
  await hold;
  await assert.rejects(pending, { code: 'check-in-expired' });
  assert.equal(calls, 0);
  client.dispose();
});

test('chat drops a reply when access changes while the RPC is in flight', async () => {
  let finish!: (value: ChatReply) => void;
  let allowed = true;
  const bridge = {
    chat: () =>
      new Promise<ChatReply>((resolve) => {
        finish = resolve;
      }),
    cancelChat: async () => {},
  } as unknown as Bridge;
  const client = chatClient(bridge, 'another-profile', 't');
  const pending = client.request({ action: 'pending' }, undefined, () => {
    if (!allowed) throw { code: 'vault-unavailable' };
  });
  await new Promise((resolve) => setImmediate(resolve));
  allowed = false;
  finish(reply);
  await assert.rejects(pending, { code: 'vault-unavailable' });
  client.dispose();
});
