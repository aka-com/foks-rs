import assert from 'node:assert/strict';
import test from 'node:test';
import type { ChatMessage, ChatOperation } from '../src/chat-contract';
import { newOutgoing } from '../src/chat/send/outgoing';
import type { TrackedOperation } from '../src/chat/operations';
import { senderName } from '../src/chat/presentation';
import {
  newMessagesUnloaded,
  splitThreadRows,
  threadRows,
  type ThreadRow,
} from '../src/chat/thread-rows';

const noon = new Date(2026, 8, 20, 12).valueOf();
const nextDay = new Date(2026, 8, 21, 9).valueOf();
const now = new Date(2026, 8, 21, 18).valueOf();
const alice = 'aa'.repeat(16);
const bob = 'bb'.repeat(16);
const names = new Map([[alice, 'alice']]);

function message(
  sequence: number,
  at: number,
  sender: string | null = alice,
): ChatMessage {
  return {
    id: `m${sequence}`,
    sequence: String(sequence),
    sender,
    send_time: String(at),
    insert_time: String(at),
    content: { kind: 'text', text: `message ${sequence}` },
  };
}

function operation(id: string): ChatOperation {
  return {
    id,
    channel: 'c',
    kind: 'send-message',
    state: 'prepared',
    confirmation: null,
    rejection_code: null,
  };
}

/** Each row as its kind, with the author and run state of message rows. */
function shape(rows: readonly ThreadRow[]): string[] {
  return rows.map((row) => {
    switch (row.kind) {
      case 'day':
        return `day ${row.label}`;
      case 'new':
        return 'new';
      case 'message':
        return `${row.message.sequence} ${row.author}${row.grouped ? ' grouped' : ''}`;
      case 'outgoing':
        return `outgoing ${row.message.id}${row.grouped ? ' grouped' : ''}`;
      case 'operation':
        return `operation ${row.operation.id}`;
    }
  });
}

test('history rows carry a separator per local day and group runs within it', () => {
  const rows = threadRows(
    [
      message(1, noon),
      message(2, noon + 60_000),
      message(3, noon + 120_000, bob),
      message(4, nextDay),
    ],
    [],
    [],
    { actor: null, newFrom: null, names, now },
  );
  assert.deepEqual(shape(rows), [
    'day Yesterday',
    '1 alice',
    '2 alice grouped',
    `3 ${senderName(bob, names)}`,
    'day Today',
    '4 alice',
  ]);
});

test('a message row carries the verification of the page that loaded it', () => {
  const rows = threadRows([message(1, noon), message(2, noon)], [], [], {
    actor: null,
    newFrom: null,
    names,
    verification: new Map([['m2', true]]),
    now,
  });
  assert.deepEqual(
    rows.flatMap((row) =>
      row.kind === 'message' ? [[row.message.id, row.incomplete]] : [],
    ),
    [
      ['m1', false],
      ['m2', true],
    ],
  );
});

test('the reader reads as You and the NEW divider starts a fresh run', () => {
  const rows = threadRows(
    [
      message(1, noon),
      message(2, noon + 60_000),
      message(3, noon + 120_000),
      message(4, noon + 180_000, bob),
    ],
    [],
    [],
    { actor: bob, newFrom: '1', names, now },
  );
  assert.deepEqual(shape(rows), [
    'day Yesterday',
    '1 alice',
    'new',
    '2 alice',
    '3 alice grouped',
    '4 You',
  ]);
  const own = rows.at(-1);
  assert.ok(own?.kind === 'message');
  assert.equal(own.own, true);
  assert.equal(own.avatarName, senderName(bob, names));
});

test('the divider is not drawn above the reader’s own message or a window that starts after it', () => {
  const messages = [message(5, noon, bob), message(6, noon + 60_000)];
  assert.deepEqual(
    shape(
      threadRows(messages, [], [], { actor: bob, newFrom: '4', names, now }),
    ),
    ['day Yesterday', '5 You', '6 alice'],
  );
  const later = [message(5, noon), message(6, noon + 60_000)];
  assert.equal(newMessagesUnloaded(later, '2', true), true);
  assert.equal(newMessagesUnloaded(later, '4', true), false);
  assert.equal(newMessagesUnloaded(later, '2', false), false);
  assert.deepEqual(
    shape(
      threadRows(later, [], [], {
        actor: null,
        newFrom: '2',
        newUnloaded: true,
        names,
        now,
      }),
    ),
    ['day Yesterday', '5 alice', '6 alice grouped'],
  );
  assert.deepEqual(
    shape(threadRows(later, [], [], { actor: null, newFrom: '4', names, now })),
    ['day Yesterday', 'new', '5 alice', '6 alice grouped'],
  );
});

test('unsent rows follow history; an operation an outgoing message carries is not repeated', () => {
  const carried = operation('op1');
  const prepared = newOutgoing({
    channel: 'c',
    submission: 's1',
    createdAt: noon,
    delivery: { stage: 'prepared', op: carried },
  });
  const queued = newOutgoing({
    channel: 'c',
    submission: 's2',
    createdAt: noon,
    delivery: { stage: 'queued' },
  });
  const observed = {
    ...newOutgoing({ channel: 'c', submission: 's3', createdAt: noon }),
    observed: true,
  };
  const operations: TrackedOperation[] = [carried, operation('op2')];
  const rows = threadRows(
    [message(1, noon)],
    [prepared, observed, queued],
    operations,
    {
      actor: alice,
      newFrom: null,
      names,
      keyOf: (m) => `key-${m.id}`,
      now,
    },
  );
  assert.deepEqual(shape(rows), [
    'day Yesterday',
    '1 You',
    'outgoing s1',
    'outgoing s2 grouped',
    'operation op2',
  ]);
  assert.deepEqual(
    rows.map((row) => row.key),
    ['day:key-m1', 'key-m1', 'outgoing:s1', 'outgoing:s2', 'operation:op2'],
  );
  const outgoing = rows.find((row) => row.kind === 'outgoing');
  assert.equal(outgoing?.kind === 'outgoing' && outgoing.avatarName, 'alice');
  assert.deepEqual(shape(splitThreadRows(rows).unsent), [
    'outgoing s1',
    'outgoing s2 grouped',
    'operation op2',
  ]);
  // With nothing unsent, every row is history.
  const history = threadRows([message(1, noon)], [], [], {
    actor: alice,
    newFrom: null,
    names,
    now,
  });
  assert.deepEqual(splitThreadRows(history), { held: history, unsent: [] });
});

test('an unsent row from a later local day than the row above it takes a day separator', () => {
  const earlier = newOutgoing({
    channel: 'c',
    submission: 's1',
    createdAt: noon + 60_000,
    delivery: { stage: 'queued' },
  });
  const later = newOutgoing({
    channel: 'c',
    submission: 's2',
    createdAt: nextDay,
    delivery: { stage: 'queued' },
  });
  const again = newOutgoing({
    channel: 'c',
    submission: 's3',
    createdAt: nextDay + 60_000,
    delivery: { stage: 'queued' },
  });
  const rows = threadRows([message(1, noon)], [earlier, later, again], [], {
    actor: alice,
    newFrom: null,
    names,
    now,
  });
  assert.deepEqual(shape(rows), [
    'day Yesterday',
    '1 You',
    'outgoing s1',
    'day Today',
    'outgoing s2',
    'outgoing s3 grouped',
  ]);
  assert.equal(rows[3].key, 'day:outgoing:s2');
  // The list draws the newer-history edge between the two sections, so an
  // unsent row's separator stays with it, below that edge.
  const { held, unsent } = splitThreadRows(rows);
  assert.deepEqual(shape(held), ['day Yesterday', '1 You']);
  assert.deepEqual(shape(unsent), [
    'outgoing s1',
    'day Today',
    'outgoing s2',
    'outgoing s3 grouped',
  ]);
  // With no history loaded, the first unsent row carries its own day.
  assert.deepEqual(
    shape(
      threadRows([], [later], [], { actor: alice, newFrom: null, names, now }),
    ),
    ['day Today', 'outgoing s2'],
  );
  const alone = splitThreadRows(
    threadRows([], [later], [], { actor: alice, newFrom: null, names, now }),
  );
  assert.deepEqual(shape(alone.held), []);
  assert.deepEqual(shape(alone.unsent), ['day Today', 'outgoing s2']);
});

test('a sender off the roster reads as a shortened id, and a missing one as a team member', () => {
  assert.equal(senderName(alice, names), 'alice');
  assert.notEqual(senderName(bob, names), bob);
  assert.ok(senderName(bob, names).startsWith('bbbb'));
  assert.equal(senderName(null, names), 'Team member');
});
