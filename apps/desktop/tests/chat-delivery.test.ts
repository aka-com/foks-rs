import assert from 'node:assert/strict';
import test from 'node:test';
import type { ChatOperation } from '../src/chat-contract';
import {
  deliveryLabel,
  deliveryNeedsAttention,
  operationDelivery,
  outgoingDelivery,
  rejectionReason,
  rejectionStatusName,
} from '../src/chat/delivery';
import { chatErrorText } from '../src/chat/error-copy';
import { failure } from '../src/chat/actions';
import {
  newOutgoing,
  transition,
  type OutgoingEvent,
  type OutgoingMessage,
} from '../src/chat/send/outgoing';

const op = (
  state: ChatOperation['state'],
  kind: ChatOperation['kind'] = 'send-message',
): ChatOperation => ({
  id: 'a'.repeat(32),
  channel: 'b'.repeat(32),
  kind,
  state,
  confirmation:
    state === 'confirmed' ? { kind: 'message-sent', sequence: '1' } : null,
  rejection_code: state === 'rejected' ? 12006 : null,
});
const run = (message: OutgoingMessage, ...events: OutgoingEvent[]) =>
  events.reduce(transition, message);
const fresh = newOutgoing({
  channel: 'b'.repeat(32),
  submission: 'c'.repeat(32),
  text: 'hello',
  createdAt: 0,
});
const prepared = run(
  fresh,
  { type: 'started', step: 'save' },
  { type: 'saving', token: 1 },
  { type: 'saved', token: 1 },
  { type: 'preparing', token: 1 },
  { type: 'prepared', token: 1, op: op('prepared'), check: false },
);

test('tracked operations and outgoing messages share one state vocabulary', () => {
  assert.equal(operationDelivery(op('prepared')), 'not-sent');
  assert.equal(operationDelivery(op('uncertain')), 'unconfirmed');
  assert.equal(operationDelivery(op('rejected')), 'rejected');
  assert.equal(operationDelivery(op('cancelled')), 'cancelled');
  assert.equal(operationDelivery(op('confirmed')), null);
  // An unknown status is checked rather than retried, until it is terminal.
  assert.equal(
    operationDelivery({ ...op('prepared'), statusUnknown: true }),
    'checking',
  );
  assert.equal(
    operationDelivery({ ...op('rejected'), statusUnknown: true }),
    'rejected',
  );

  assert.equal(outgoingDelivery(run(fresh, { type: 'queued' })), 'queued');
  assert.equal(outgoingDelivery(prepared), 'sending');
  const settled = run(prepared, { type: 'finished', token: 1 });
  assert.equal(outgoingDelivery(settled), 'not-sent');
  // A rejected message reads as refused, not as merely not sent.
  assert.equal(
    outgoingDelivery(
      run(
        prepared,
        { type: 'attempted', token: 1, op: op('rejected') },
        { type: 'finished', token: 1 },
      ),
    ),
    'rejected',
  );
  assert.equal(
    outgoingDelivery(
      run(
        prepared,
        { type: 'attempted', token: 1, op: op('confirmed') },
        { type: 'finished', token: 1 },
      ),
    ),
    null,
  );
  const uncertain = run(
    prepared,
    { type: 'attempted', token: 1, op: op('uncertain') },
    { type: 'finished', token: 1 },
  );
  assert.equal(outgoingDelivery(uncertain), 'unconfirmed');
  assert.equal(
    outgoingDelivery(run(uncertain, { type: 'started', step: 'check' })),
    'checking',
  );

  assert.deepEqual(
    (
      [
        'sending',
        'queued',
        'not-sent',
        'checking',
        'unconfirmed',
        'rejected',
        'cancelled',
        'paused',
      ] as const
    ).map((state) => deliveryLabel(state)),
    [
      'Sending',
      'Queued',
      'Not sent',
      'Checking delivery',
      'Couldn’t confirm delivery',
      'Couldn’t post',
      'Cancelled',
      'Waiting for access',
    ],
  );
  assert.equal(deliveryLabel('rejected', 'create-channel'), 'Couldn’t create');
  assert.deepEqual(
    (['sending', 'queued', 'cancelled', null] as const).map(
      deliveryNeedsAttention,
    ),
    [false, false, false, false],
  );
  assert.deepEqual(
    (
      ['not-sent', 'checking', 'unconfirmed', 'rejected', 'paused'] as const
    ).map(deliveryNeedsAttention),
    [true, true, true, true, true],
  );
});

test('rejection codes read as reasons, and only Details names the status', () => {
  // The closed set of `definite_rejection` in foks-client.
  for (const code of [1013, 1030, 12002, 12003, 12005, 12006])
    assert.match(rejectionStatusName(code) ?? '', /^STATUS_/);
  assert.equal(rejectionStatusName(12001), undefined);
  assert.equal(rejectionStatusName(null), undefined);
  assert.equal(
    rejectionReason(12006, 'send-message', '#general'),
    'New messages arrived in #general first.',
  );
  assert.equal(
    rejectionReason(1013, 'send-message', '#incidents'),
    'You can no longer post in #incidents.',
  );
  assert.equal(
    rejectionReason(12005, 'send-message'),
    'This channel no longer exists.',
  );
  assert.equal(
    rejectionReason(12002, 'create-channel', 'Engineering'),
    'Engineering already has a channel with that name.',
  );
  assert.equal(
    rejectionReason(4242, 'send-message'),
    'The server refused the message.',
  );
  for (const code of [1013, 1030, 12002, 12003, 12005, 12006, 4242, null])
    for (const kind of ['send-message', 'create-channel'] as const)
      assert.doesNotMatch(rejectionReason(code, kind), /\d/);
});

test('chat failures from the agent show the copy table, not the agent text', () => {
  const agent = {
    code: 'chat-unsupported',
    message:
      'unsupported chat operation: latest message has unsupported content',
    retryable: false,
    ambiguous: false,
    fatal: false,
    origin: 'invoke' as const,
  };
  assert.equal(
    chatErrorText(agent),
    'This team or channel uses a chat feature this version of FOKS does not support.',
  );
  assert.equal(failure(agent), chatErrorText(agent));
  assert.equal(
    chatErrorText({ ...agent, code: 'chat-channel-integrity' }),
    'Content in this channel could not be verified.',
  );
  // The desktop's own failures keep their message, which it wrote for the
  // reader, and so do codes outside the table.
  const local = {
    ...agent,
    code: 'chat-limit',
    message: 'This conversation view reached its history limit.',
    origin: undefined,
  };
  assert.equal(chatErrorText(local), local.message);
  assert.equal(
    chatErrorText({ ...agent, code: 'server-unavailable', message: 'Offline' }),
    'Offline',
  );
});
