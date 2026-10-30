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
    // Once `submit-message` is issued, preparation remains ambiguous and the
    // phase is `unconfirmed` rather than `paused`.
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
    // Access changed after the save was acknowledged; restoring the draft
    // clears the saved intent.
    'saved, then access changed + intent-cleared',
    run(saving, { type: 'saved', token: 1 }, fail(1, true), {
      type: 'finished',
      token: 1,
    }),
    { type: 'intent-cleared' },
    'paused',
    (next) => {
      assert.equal(next.intent, 'cleared');
      // With the saved intent cleared, the message is unsent in memory only.
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
    // When chat access is unavailable and no drive is running, the phase is
    // `paused` and `awaitingAccess` is true so recovery reconciles once access
    // returns.
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
    // The phase is `paused`, while `outcomeUnknown` remains true so the
    // message is reconciled rather than edited.
    (next) => assert.equal(outcomeUnknown(next), true),
  ],
  [
    // While a `check` drive is running, `phaseOf` returns `unconfirmed`.
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
    // A non-terminal status report does not reopen a terminal operation.
    (next) => assert.equal(next, confirmed),
  ],
  [
    // A status reply confirmed the operation while the drive's attempt was in flight.
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
    // `transition` is pure and does not mutate its input state.
    assert.deepEqual(state, before);
    check?.(next);
  });

// Events carrying token 1 from the first drive are ignored when the current
// attempt token is 2.
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
  // Drive 1 failed during `attempt` and requested `status`. Retry starts
  // drive 2 (`check`) before that status reply arrives.
  let message = run(attemptFailed, { type: 'started', step: 'check' });
  message = transition(message, { type: 'status', op: op('prepared') });
  assert.equal(running(message), true);
  assert.equal(phaseOf(message), 'unconfirmed');
  assert.equal(message.stopped, null);
  // Stale events from drive 1 do not affect drive 2.
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
