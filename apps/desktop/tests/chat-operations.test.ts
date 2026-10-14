import assert from 'node:assert/strict';
import test from 'node:test';
import {
  delivered,
  observeMessages,
  reconcileOperations as reduceOperations,
} from '../src/chat/operations';
import type {
  ChatAction,
  ChatOperation,
  ChatResult,
} from '../src/chat-contract';
import { eventFromReply, historyEvent } from '../src/chat/conversation-events';
import { mergeHistoryPage } from '../src/chat/conversation-model';
import type { TrackedOperation } from '../src/chat/operations';
const reconcileOperations = (
  old: TrackedOperation[],
  action: ChatAction,
  result: ChatResult,
) => reduceOperations(old, eventFromReply(action, result));
const page = (result: ChatResult, before: string | null = null) => {
  if (result.kind !== 'history') throw new Error('history expected');
  return historyEvent(result, before);
};
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
  if (history.kind !== 'history') throw new Error('fixture');
  const rows = observeMessages(
    [{ ...op, text: 'hello' }],
    delivered(history.messages),
  );
  const refreshed = reconcileOperations(
    rows,
    { action: 'pending' },
    { kind: 'pending', operations: [op] },
  );
  assert.equal(refreshed[0].observed, true);
  assert.equal(refreshed[0].text, undefined);
});

test('a pending reply after accepted history keeps the send observed, and a conflicting page is refused', () => {
  const window = mergeHistoryPage(null, page(history));
  const rows = observeMessages(
    reconcileOperations(
      [],
      { action: 'pending' },
      { kind: 'pending', operations: [op] },
    ),
    delivered(window.messages),
  );
  assert.equal(rows[0].observed, true);
  assert.equal(window.messages.length, 1);
  if (history.kind !== 'history') throw new Error('fixture');
  const conflict = {
    ...history,
    messages: history.messages.map((m) => ({
      ...m,
      content: { kind: 'text' as const, text: 'changed' },
    })),
  };
  assert.throws(() => mergeHistoryPage(window, page(conflict)), {
    code: 'chat-channel-integrity',
  });
  // A different row at a held sequence is refused, and the held window is
  // unchanged.
  assert.throws(
    () =>
      mergeHistoryPage(
        window,
        page({
          ...history,
          messages: [{ ...history.messages[0], id: 'e'.repeat(32) }],
        }),
      ),
    { code: 'chat-channel-integrity' },
  );
  assert.deepEqual(window.messages, history.messages);
});

test('an unverifiable row that a later read opens is replaced, not a conflict', () => {
  if (history.kind !== 'history') throw new Error('fixture');
  const row = history.messages[0];
  const unverifiable = {
    ...history,
    messages: [{ ...row, content: { kind: 'unverifiable' as const } }],
  };
  // An unverifiable row bearing a sent message's ID is no delivery evidence.
  const held = mergeHistoryPage(null, page(unverifiable));
  const sends = observeMessages(
    [{ ...op, text: 'hello' }],
    delivered(unverifiable.messages),
  );
  assert.equal(sends[0].observed, undefined);
  assert.equal(sends[0].text, 'hello');
  // The reader's team view now holds the key generation the row names.
  const opened = mergeHistoryPage(held, page(history));
  assert.deepEqual(opened.messages, history.messages);
  assert.equal(
    observeMessages(sends, delivered(history.messages))[0].observed,
    true,
  );
  // A row never goes back to unverifiable, and the server's claims about an
  // unverifiable row cannot change either.
  assert.throws(() => mergeHistoryPage(opened, page(unverifiable)), {
    code: 'chat-channel-integrity',
  });
  assert.throws(
    () =>
      mergeHistoryPage(
        held,
        page({
          ...history,
          messages: [{ ...row, insert_time: '2' }],
        }),
      ),
    { code: 'chat-channel-integrity' },
  );
});

test('unverifiable rows keyed by sequence coexist across pages', () => {
  const row = (sequence: string) => ({
    id: `zero:${sequence}`,
    sequence,
    sender: null,
    send_time: '1',
    insert_time: '1',
    content: { kind: 'unverifiable' as const },
  });
  const result = (sequence: string, before: string | null): ChatResult => ({
    kind: 'history',
    channel: op.channel,
    before,
    missing_predecessors: [],
    messages: [row(sequence)],
  });
  const newest = mergeHistoryPage(null, page(result('9', '9')));
  const both = mergeHistoryPage(newest, page(result('8', '8'), '9'));
  assert.deepEqual(
    both.messages.map((m) => m.id),
    ['zero:8', 'zero:9'],
  );
});

test('a page for another channel replaces the held window', () => {
  const held = mergeHistoryPage(null, page(history));
  if (history.kind !== 'history') throw new Error('fixture');
  const other = mergeHistoryPage(
    held,
    page({
      ...history,
      channel: 'c'.repeat(32),
      messages: [{ ...history.messages[0], id: 'd'.repeat(32) }],
    }),
  );
  assert.equal(other.channel, 'c'.repeat(32));
  assert.deepEqual(
    other.messages.map((m) => m.id),
    ['d'.repeat(32)],
  );
});
