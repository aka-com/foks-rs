import assert from 'node:assert/strict';
import test from 'node:test';
import { reconcileOperations } from '../src/chat/operations';
import type { ChatOperation, ChatResult } from '../src/chat-contract';
const op: ChatOperation = {
  id: 'a'.repeat(32),
  channel: 'b'.repeat(32),
  create: false,
  state: 'prepared',
  sequence: null,
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
      operation: { ...op, state: 'confirmed', sequence: '1' },
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
  const { conversationResult, emptyConversation } =
    await import('../src/chat/conversation-model');
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
