import assert from 'node:assert/strict';
import test from 'node:test';
import type { ChatOperation } from '../src/chat-contract';
import {
  awaitingAccess,
  cleanupErrorOf,
  copyOutgoing,
  intentHeld,
  newOutgoing,
  operationOf,
  outcomeUnknown,
  outgoingActions,
  phaseOf,
  preparationAmbiguous,
  running,
  transition,
  unsent,
  type OutgoingEvent,
  type OutgoingMessage,
  type SendPhase,
} from '../src/chat/send/outgoing';

const op = (state: ChatOperation['state']): ChatOperation => ({
  id: 'a'.repeat(32),
  channel: 'b'.repeat(32),
  kind: 'send-message',
  state,
  confirmation:
    state === 'confirmed' ? { kind: 'message-sent', sequence: '1' } : null,
  rejection_code: state === 'rejected' ? 12006 : null,
});
const fail = (token: number, access = false): OutgoingEvent => ({
  type: 'failed',
  token,
  error: 'Temporarily unavailable',
  access,
});
const run = (message: OutgoingMessage, ...events: OutgoingEvent[]) =>
  events.reduce(transition, message);

const fresh = newOutgoing({
  channel: 'b'.repeat(32),
  submission: 'c'.repeat(32),
  text: 'hello',
  createdAt: 0,
});
const queued = run(fresh, { type: 'queued' });
const saving = run(
  fresh,
  { type: 'started', step: 'save' },
  {
    type: 'saving',
    token: 1,
  },
);
const submitting = run(
  saving,
  { type: 'saved', token: 1 },
  { type: 'preparing', token: 1 },
);
const submitLost = run(submitting, fail(1), {
  type: 'finished',
  token: 1,
});
const delivering = run(submitting, {
  type: 'prepared',
  token: 1,
  op: op('prepared'),
  check: false,
});
const attemptFailed = run(delivering, fail(1), { type: 'finished', token: 1 });
const confirmed = run(
  delivering,
  { type: 'attempted', token: 1, op: op('confirmed') },
  { type: 'finished', token: 1 },
);

const table: [
  name: string,
  state: OutgoingMessage,
  event: OutgoingEvent,
  phase: SendPhase,
  check?: (next: OutgoingMessage) => void,
][] = [
  ['fresh + queued', fresh, { type: 'queued' }, 'queued'],
  [
    'queued + waiting',
    queued,
    { type: 'waiting' },
    'paused',
    (next) => assert.equal(next.delivery.stage, 'saving'),
  ],
  [
    'queued + started',
    queued,
    { type: 'started', step: 'save' },
    'saving',
    (next) => {
      assert.equal(next.attempt.token, 1);
      assert.equal(intentHeld(next), false);
    },
  ],
  [
    'started + saving',
    run(fresh, { type: 'started', step: 'save' }),
    { type: 'saving', token: 1 },
    'saving',
    (next) => {
      assert.equal(next.intent, 'pending');
      assert.equal(unsent(next), true);
    },
  ],
  [
    'saving + saved',
    saving,
    { type: 'saved', token: 1 },
    'saving',
    (next) => {
      assert.deepEqual(next.delivery, {
        stage: 'preparing',
        ambiguous: false,
      });
      assert.equal(unsent(next), false);
    },
  ],
  [
    'saving + failed',
    saving,
    fail(1),
    'not-sent',
    // A lost save acknowledgement never makes delivery ambiguous.
    (next) => assert.equal(preparationAmbiguous(next), false),
  ],
  ['saving + failed with access changed', saving, fail(1, true), 'paused'],
  [
    'saved + preparing',
    run(saving, { type: 'saved', token: 1 }),
    { type: 'preparing', token: 1 },
    'preparing',
    (next) => assert.equal(preparationAmbiguous(next), true),
  ],
  ['submitting + failed', submitting, fail(1), 'unconfirmed'],
  [
    // The submit was issued, so the preparation stays ambiguous and does not
    // read as waiting for access.
    'submitting + failed with access changed',
    submitting,
    fail(1, true),
    'unconfirmed',
  ],
  [
    'ambiguous + queued',
    submitLost,
    { type: 'queued' },
    'unconfirmed',
    (next) => assert.equal(next, submitLost),
  ],
  [
    'ambiguous + intent-cleared',
    submitLost,
    { type: 'intent-cleared' },
    'unconfirmed',
    (next) => assert.equal(preparationAmbiguous(next), true),
  ],
  [
    // Access changed after the save was acknowledged; Edit clears the copy.
    'saved, then access changed + intent-cleared',
    run(saving, { type: 'saved', token: 1 }, fail(1, true), {
      type: 'finished',
      token: 1,
    }),
    { type: 'intent-cleared' },
    'paused',
    (next) => {
      assert.equal(next.intent, 'cleared');
      // The agent no longer holds the copy, so closing the window loses it.
      assert.equal(unsent(next), true);
    },
  ],
  [
    'submitting + prepared',
    submitting,
    { type: 'prepared', token: 1, op: op('prepared'), check: false },
    'sending',
    (next) => assert.equal(operationOf(next)?.state, 'prepared'),
  ],
  [
    'submitting + prepared uncertain',
    submitting,
    { type: 'prepared', token: 1, op: op('uncertain'), check: true },
    'unconfirmed',
  ],
  [
    'delivering + attempted confirmed',
    delivering,
    { type: 'attempted', token: 1, op: op('confirmed') },
    'sent',
  ],
  [
    'delivering + attempted rejected',
    delivering,
    { type: 'attempted', token: 1, op: op('rejected') },
    'not-sent',
  ],
  [
    'delivering + failed',
    delivering,
    fail(1),
    'unconfirmed',
    (next) => assert.equal(next.delivery.stage, 'uncertain'),
  ],
  [
    'attempt failed + status prepared',
    attemptFailed,
    { type: 'status', op: op('prepared') },
    'not-sent',
  ],
  [
    'attempt failed with access changed + status prepared',
    run(delivering, fail(1, true), { type: 'finished', token: 1 }),
    { type: 'status', op: op('prepared') },
    'paused',
  ],
  [
    'attempt failed + status uncertain',
    attemptFailed,
    { type: 'status', op: op('uncertain') },
    'unconfirmed',
  ],
  [
    'attempt failed + status confirmed',
    attemptFailed,
    { type: 'status', op: op('confirmed') },
    'sent',
    (next) => assert.equal(next.error, ''),
  ],
  [
    'attempt failed + status of another operation',
    attemptFailed,
    { type: 'status', op: { ...op('confirmed'), id: 'd'.repeat(32) } },
    'unconfirmed',
    (next) => assert.equal(next, attemptFailed),
  ],
  [
    'attempt failed + started a check',
    attemptFailed,
    { type: 'started', step: 'check' },
    'unconfirmed',
    (next) => {
      assert.equal(next.error, '');
      assert.equal(next.stopped, null);
    },
  ],
  [
    'cancelled + status',
    attemptFailed,
    { type: 'status', op: op('cancelled') },
    'cancelled',
  ],
  [
    'confirmed with a saved copy + observed',
    confirmed,
    { type: 'observed' },
    'sent',
    // The text stays until the saved copy is cleared.
    (next) => assert.equal(next.text, 'hello'),
  ],
  [
    'observed with a saved copy + intent-cleared',
    run(confirmed, { type: 'observed' }),
    { type: 'intent-cleared' },
    'sent',
    (next) => assert.equal(next.text, undefined),
  ],
  [
    'confirmed + cleanup-failed',
    confirmed,
    { type: 'cleanup-failed', error: 'Disk full' },
    'sent',
    (next) => {
      assert.equal(cleanupErrorOf(next), 'Disk full');
      assert.equal(intentHeld(next), true);
    },
  ],
  [
    'submitting + revoked',
    submitting,
    { type: 'revoked' },
    'paused',
    (next) => {
      assert.equal(next.text, undefined);
      assert.equal(running(next), true);
    },
  ],
  [
    'revoked during delivery + attempted prepared',
    run(delivering, { type: 'revoked' }),
    { type: 'attempted', token: 1, op: op('prepared') },
    'sending',
    (next) => {
      assert.equal(next.stopped, null);
      assert.equal(
        phaseOf(transition(next, { type: 'finished', token: 1 })),
        'not-sent',
      );
    },
  ],
  [
    // Check again cannot act while chat is unavailable, so the row says it is
    // waiting; recovery checks it once access returns.
    'attempt failed + waiting',
    attemptFailed,
    { type: 'waiting' },
    'paused',
    (next) => assert.equal(awaitingAccess(next), true),
  ],
  [
    'attempt failed with access changed + finished',
    run(delivering, fail(1, true)),
    { type: 'finished', token: 1 },
    'paused',
    // It reads as waiting, but is still checked rather than edited.
    (next) => assert.equal(outcomeUnknown(next), true),
  ],
  [
    // A drive checking the operation is running, so the row shows the check.
    'unconfirmed and checking + revoked',
    run(attemptFailed, { type: 'started', step: 'check' }),
    { type: 'revoked' },
    'unconfirmed',
  ],
  [
    'ambiguous with its copy cleared + queued',
    run(submitLost, { type: 'intent-cleared' }),
    { type: 'queued' },
    'unconfirmed',
    (next) => assert.equal(preparationAmbiguous(next), true),
  ],
  ['confirmed + revoked', confirmed, { type: 'revoked' }, 'sent'],
  [
    'confirmed + status prepared',
    confirmed,
    { type: 'status', op: op('prepared') },
    'sent',
    // A terminal operation is never reopened by a report.
    (next) => assert.equal(next, confirmed),
  ],
  [
    // A status reply confirmed it while the drive's attempt was in flight.
    'confirmed during delivery + attempted prepared',
    run(delivering, { type: 'status', op: op('confirmed') }),
    { type: 'attempted', token: 1, op: op('prepared') },
    'sent',
  ],
  [
    'submitting + retired',
    submitting,
    { type: 'retired' },
    'preparing',
    (next) => assert.equal(next.text, undefined),
  ],
  [
    'observed + body',
    run(confirmed, { type: 'observed' }, { type: 'intent-cleared' }),
    { type: 'body', text: 'hello' },
    'sent',
    (next) => assert.equal(next.text, undefined),
  ],
];

for (const [name, state, event, phase, check] of table)
  test(`outgoing transition: ${name}`, () => {
    const before = structuredClone(state);
    const next = transition(state, event);
    assert.equal(phaseOf(next), phase);
    // The transition is pure: the state it was given is unchanged.
    assert.deepEqual(state, before);
    check?.(next);
  });

// Each event carries the first drive's token, 1; the state is the second's.
for (const event of [
  { type: 'saving', token: 1 },
  { type: 'saved', token: 1 },
  { type: 'preparing', token: 1 },
  { type: 'prepared', token: 1, op: op('prepared'), check: false },
  { type: 'attempted', token: 1, op: op('confirmed') },
  fail(1),
  { type: 'finished', token: 1 },
] satisfies OutgoingEvent[])
  test(`outgoing transition: a ${event.type} event from an earlier drive is discarded`, () => {
    const state = run(attemptFailed, { type: 'started', step: 'check' });
    assert.equal(state.attempt.token, 2);
    assert.equal(transition(state, event), state);
  });

test('outgoing transition: a status reply after Retry does not end or redirect the Retry', () => {
  // The first drive failed at its attempt and asked for the operation's
  // status. Retry starts a second drive before that reply arrives.
  let message = run(attemptFailed, { type: 'started', step: 'check' });
  message = transition(message, { type: 'status', op: op('prepared') });
  assert.equal(running(message), true);
  assert.equal(phaseOf(message), 'unconfirmed');
  assert.equal(message.stopped, null);
  // A late continuation of the first drive cannot end the second one.
  message = run(message, fail(1), { type: 'finished', token: 1 });
  assert.equal(running(message), true);
  assert.equal(message.error, '');
  message = run(
    message,
    { type: 'attempted', token: 2, op: op('confirmed') },
    { type: 'finished', token: 2 },
  );
  assert.equal(running(message), false);
  assert.equal(phaseOf(message), 'sent');
});

const rejected = run(
  delivering,
  { type: 'attempted', token: 1, op: op('rejected') },
  { type: 'finished', token: 1 },
);
const actionTable: [
  name: string,
  state: OutgoingMessage,
  retry: 'retry' | 'check' | undefined,
  edit: boolean,
][] = [
  ['queued', queued, undefined, true],
  ['saving', saving, undefined, false],
  [
    'save failed',
    run(saving, fail(1), { type: 'finished', token: 1 }),
    'retry',
    true,
  ],
  [
    'save failed with access changed',
    run(saving, fail(1, true), { type: 'finished', token: 1 }),
    'retry',
    false,
  ],
  // A preparation that may have reached the agent is checked, never edited.
  ['submit reply lost', submitLost, 'check', false],
  ['delivering', delivering, undefined, false],
  ['attempt failed', attemptFailed, 'check', false],
  [
    'attempt failed with access changed',
    run(delivering, fail(1, true), { type: 'finished', token: 1 }),
    'check',
    false,
  ],
  // The server refused it, so it is edited rather than sent again.
  ['rejected', rejected, undefined, true],
  ['confirmed', confirmed, undefined, false],
];
for (const [name, state, retry, edit] of actionTable)
  test(`outgoing actions: ${name}`, () => {
    assert.deepEqual(outgoingActions(state), { retry, edit });
  });

test('outgoing actions: an operation the device lost track of is checked, never edited', () => {
  // The status read after the failed attempt reports the operation prepared.
  const reported = run(attemptFailed, { type: 'status', op: op('prepared') });
  assert.deepEqual(outgoingActions(reported), { retry: 'retry', edit: true });
  // A later failed request left the tracked operation's status unknown, which
  // the message itself does not record.
  assert.deepEqual(outgoingActions(reported, true), {
    retry: 'check',
    edit: false,
  });
});

test('outgoing copy shares no mutable state with the message', () => {
  const message = run(attemptFailed, {
    type: 'cleanup-failed',
    error: 'Disk full',
  });
  const copy = copyOutgoing(message);
  assert.deepEqual(copy, message);
  assert.notEqual(copy.delivery, message.delivery);
  assert.notEqual(operationOf(copy), operationOf(message));
  assert.notEqual(copy.intent, message.intent);
  assert.notEqual(copy.attempt, message.attempt);
});
