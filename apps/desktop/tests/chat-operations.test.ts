import assert from 'node:assert/strict';
import test from 'node:test';
import { reconcileOperations as reduceOperations } from '../src/chat/operations';
import type {
  ChatAction,
  ChatOperation,
  ChatResult,
} from '../src/chat-contract';
import { eventFromReply } from '../src/chat/conversation-events';
import {
  conversationResult as reduceConversation,
  emptyConversation,
} from '../src/chat/conversation-model';
import type { ConversationState } from '../src/chat/conversation-model';
import type { TrackedOperation } from '../src/chat/operations';
const reconcileOperations = (
  old: TrackedOperation[],
  action: ChatAction,
  result: ChatResult,
) => reduceOperations(old, eventFromReply(action, result));
const conversationResult = (
  state: ConversationState,
  action: ChatAction,
  result: ChatResult,
) => reduceConversation(state, eventFromReply(action, result));
const op: ChatOperation = {
  id: 'a'.repeat(32),
  channel: 'b'.repeat(32),
  kind: 'send-message',
  state: 'prepared',
  confirmation: null,
  rejection_code: null,
};
const history: ChatResult = {
  kind: 'history',
  channel: op.channel,
  before: null,
  missing_predecessors: [],
  messages: [
    {
      id: op.id,
      sequence: '1',
      sender: null,
      send_time: '1',
      insert_time: '1',
      content: { kind: 'text', text: 'hello' },
    },
  ],
};
test('body recovery never resurrects observed text or changes delivery status', () => {
  const event = {
    kind: 'body-recovered',
    operation: op.id,
    channel: op.channel,
    text: 'original',
  } as const;
  const recovered = reduceOperations([{ ...op, statusUnknown: true }], event);
  assert.equal(recovered[0].text, 'original');
  assert.equal(recovered[0].statusUnknown, true);
  assert.equal(recovered[0].state, 'prepared');
  const absent = reduceOperations([op], { ...event, text: null });
  assert.equal(absent[0].bodyUnavailable, true);
  assert.equal(absent[0].state, 'prepared');
  assert.equal(absent[0].id, op.id);
  assert.equal(
    reduceOperations([{ ...op, observed: true }], event)[0].text,
    undefined,
  );
  assert.equal(
    reduceOperations([op], { ...event, channel: 'f'.repeat(32) })[0].text,
    undefined,
  );
});
test('pending omission preserves unknown send but does not recheck terminal state', () => {
  const rows = reconcileOperations(
    [{ ...op, text: 'hello' }],
    { action: 'pending' },
    { kind: 'pending', operations: [] },
  );
  assert.equal(rows[0].text, 'hello');
  assert.equal(rows[0].statusUnknown, true);
  const confirmed = reconcileOperations(
    rows,
    { action: 'status', operation: op.id },
    {
      kind: 'operation',
      operation: {
        ...op,
        state: 'confirmed',
        confirmation: { kind: 'message-sent', sequence: '1' },
      },
    },
  );
  assert.equal(
    reconcileOperations(
      confirmed,
      { action: 'pending' },
      { kind: 'pending', operations: [] },
    )[0].statusUnknown,
    false,
  );
});
test('history observation survives later ledger replies without retaining plaintext', () => {
  const rows = reconcileOperations(
    [{ ...op, text: 'hello' }],
    { action: 'history', channel: op.channel, before: null },
    history,
  );
  const refreshed = reconcileOperations(
    rows,
    { action: 'pending' },
    { kind: 'pending', operations: [op] },
  );
  assert.equal(refreshed[0].observed, true);
  assert.equal(refreshed[0].text, undefined);
});

test('atomic model handles history before pending and rejects conflicting pages without dropping sends', async () => {
  const action = {
    action: 'history',
    channel: op.channel,
    before: null,
  } as const;
  const accepted = conversationResult(emptyConversation(), action, history);
  const state = conversationResult(
    accepted,
    { action: 'pending' },
    { kind: 'pending', operations: [op] },
  );
  assert.equal(state.operations[0].observed, true);
  assert.equal(state.history?.messages.length, 1);
  if (history.kind !== 'history') throw new Error('fixture');
  const conflict = {
    ...history,
    messages: history.messages.map((m) => ({
      ...m,
      content: { kind: 'text' as const, text: 'changed' },
    })),
  };
  assert.throws(() => conversationResult(state, action, conflict));
  const unresolved = conversationResult(
    state,
    { action: 'pending' },
    { kind: 'pending', operations: [{ ...op, id: 'e'.repeat(32) }] },
  );
  assert.throws(() =>
    conversationResult(unresolved, action, {
      ...history,
      messages: [{ ...history.messages[0], id: 'e'.repeat(32) }],
    }),
  );
  assert.equal(
    unresolved.operations.find((o) => o.id === 'e'.repeat(32))?.observed,
    undefined,
  );
});
