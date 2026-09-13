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

test('poll bypasses queued work and only history may request background admission', async () => {
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
  release();
  await hold;
  client.dispose();
});
