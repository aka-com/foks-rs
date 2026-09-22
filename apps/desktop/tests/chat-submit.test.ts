import assert from 'node:assert/strict';
import test from 'node:test';
import { mockChat } from '../src/chat-mock';
import type { ChatAction, ChatOperation } from '../src/chat-contract';

const channel = 'ab'.repeat(16);
const input = {
  action: 'submit-message',
  submission: 'ef'.repeat(16),
  channel,
  text: 'original body',
} as const satisfies ChatAction;

test('fresh submit sends once and legacy prepare recovers the same operation', async () => {
  const chat = mockChat();
  const sent = await chat('team', input);
  assert.equal(sent.result.kind, 'operation');
  if (sent.result.kind !== 'operation') throw new Error('operation expected');
  assert.equal(sent.result.operation.state, 'confirmed');
  assert.equal(sent.result.operation.confirmation?.kind, 'message-sent');
  assert.deepEqual(await chat('team', input), sent);
  assert.deepEqual(
    await chat('team', { ...input, action: 'prepare-message' }),
    sent,
  );
  const history = await chat('team', {
    action: 'history',
    channel,
    before: null,
  });
  if (history.result.kind !== 'history') throw new Error('history expected');
  assert.equal(history.result.messages.length, 2);
  assert.equal(history.result.messages[1].id, sent.result.operation.id);
  assert.deepEqual(history.result.messages[1].content, {
    kind: 'text',
    text: input.text,
  });
});

for (const state of [
  'prepared',
  'uncertain',
  'confirmed',
  'cancelled',
  'rejected',
] as const) {
  test(`repeat submit leaves existing ${state} operation unchanged`, async () => {
    const chat = mockChat();
    const prepared = await chat('team', {
      ...input,
      action: 'prepare-message',
    });
    if (prepared.result.kind !== 'operation')
      throw new Error('operation expected');
    const pending = await chat('team', { action: 'pending' });
    if (pending.result.kind !== 'pending') throw new Error('pending expected');
    const operation = pending.result.operations[0];
    operation.state = state;
    operation.confirmation =
      state === 'confirmed' ? { kind: 'message-sent', sequence: '2' } : null;
    operation.rejection_code = state === 'rejected' ? 1 : null;
    const before: ChatOperation = structuredClone(operation);
    const repeated = await chat('team', input);
    assert.deepEqual(repeated.result, { kind: 'operation', operation: before });
    assert.deepEqual(
      (await chat('team', { ...input, action: 'prepare-message' })).result,
      repeated.result,
    );
    const history = await chat('team', {
      action: 'history',
      channel,
      before: null,
    });
    if (history.result.kind !== 'history') throw new Error('history expected');
    assert.equal(history.result.messages.length, 1);
    if (state === 'prepared' || state === 'uncertain') {
      const body = await chat('team', {
        action: 'operation-body',
        operation: before.id,
        channel,
      });
      assert.equal(body.result.kind, 'operation-body');
      if (body.result.kind === 'operation-body')
        assert.equal(body.result.text, input.text);
    }
  });
}

for (const first of ['prepare-message', 'submit-message'] as const) {
  test(`${first} fingerprint rejects changed body and channel for either action`, async () => {
    const chat = mockChat();
    const original = await chat('team', { ...input, action: first });
    for (const action of ['prepare-message', 'submit-message'] as const) {
      for (const changed of [
        { text: 'different' },
        { channel: 'cd'.repeat(16) },
      ])
        await assert.rejects(chat('team', { ...input, ...changed, action }), {
          code: 'conflict',
        });
      assert.deepEqual(
        await chat('team', {
          text: input.text,
          channel: input.channel,
          submission: input.submission,
          action,
        }),
        original,
      );
    }
  });
}
