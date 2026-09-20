import assert from 'node:assert/strict';
import test from 'node:test';
import { chatIntentPersistence } from '../src/chat/intent';
import {
  decodeChatReply,
  type ChatReply,
  type ChatScope,
  type ChatAction,
} from '../src/chat-contract';
import { mockBridge } from '../src/mock-bridge';
import type { Bridge } from '../src/bridge';

const scope: ChatScope = {
  host: '02' + 'ab'.repeat(32),
  actor: '01' + 'ab'.repeat(32),
  store: {
    profile: 'local',
    account_alias: 'owner',
    team_alias: 'team',
    team_id: '03' + 'cd'.repeat(32),
  },
};
const storeId = JSON.stringify({
  kind: 'team',
  profile: scope.store.profile,
  accountAlias: scope.store.account_alias,
  teamAlias: scope.store.team_alias,
  teamId: scope.store.team_id,
});
const channel = 'ab'.repeat(16);
const saved = { submission: 'cd'.repeat(16), text: 'saved before network' };
const target = { host: scope.host, actor: scope.actor, channel };
const reply: ChatReply = {
  scope,
  result: { kind: 'intent', channel, intent: saved },
};
function bridgeReturning(value: unknown): Bridge {
  return {
    ...mockBridge(),
    chat: async (id, action) => decodeChatReply(value, id, action),
    chatLocal: async () => {
      throw new Error('Intent persistence must not use desktop credentials.');
    },
  };
}

test('agent intent decoding validates identity, bounded text and request correlation', () => {
  const action: ChatAction = { action: 'load-intent', ...target };
  assert.deepEqual(decodeChatReply(reply, storeId, action), reply);
  for (const intent of [
    { ...saved, text: '' },
    { ...saved, text: 'x'.repeat(65537) },
    { ...saved, submission: 'bad' },
  ]) {
    assert.throws(() =>
      decodeChatReply(
        { ...reply, result: { kind: 'intent', channel, intent } },
        storeId,
        action,
      ),
    );
  }
  for (const changed of [
    {
      ...reply,
      result: { kind: 'intent', channel: 'ef'.repeat(16), intent: saved },
    },
    { ...reply, scope: { ...scope, actor: '01' + 'ef'.repeat(32) } },
    {
      ...reply,
      scope: {
        ...scope,
        store: { ...scope.store, account_alias: 'replacement' },
      },
    },
  ])
    assert.throws(() => decodeChatReply(changed, storeId, action));
});

test('intent persistence checks saved input and never calls chatLocal', async () => {
  const service = (value: unknown = reply) =>
    chatIntentPersistence(bridgeReturning(value), storeId, scope, channel);
  assert.deepEqual(await service().load(), saved);
  await service().save(saved);
  for (const intent of [
    { ...saved, text: 'different' },
    { ...saved, submission: 'ef'.repeat(16) },
    null,
  ]) {
    await assert.rejects(
      service({ ...reply, result: { kind: 'intent', channel, intent } }).save(
        saved,
      ),
      { code: 'chat-integrity' },
    );
  }
  assert.equal(
    await service({
      ...reply,
      result: { kind: 'intent', channel, intent: null },
    }).load(),
    undefined,
  );
});

test('save and clear use the same immutable identity over checked agent requests', async () => {
  const calls: unknown[] = [];
  const bridge: Bridge = {
    ...mockBridge(),
    chat: async (id, action) => {
      calls.push({ id, action });
      return decodeChatReply(
        {
          scope,
          result: {
            kind: 'intent',
            channel,
            intent: action.action === 'clear-intent' ? null : saved,
          },
        },
        id,
        action,
      );
    },
  };
  const persistence = chatIntentPersistence(bridge, storeId, scope, channel);
  await persistence.save(saved);
  await persistence.clear(saved.submission);
  assert.deepEqual(calls, [
    { id: storeId, action: { action: 'save-intent', ...target, ...saved } },
    {
      id: storeId,
      action: {
        action: 'clear-intent',
        ...target,
        submission: saved.submission,
      },
    },
  ]);
});
