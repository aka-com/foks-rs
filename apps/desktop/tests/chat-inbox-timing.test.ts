import assert from 'node:assert/strict';
import test from 'node:test';
import { recordInboxTiming } from '../src/chat/inbox-timing';
import { diagnosticLog, hashId } from '../src/diagnostics/log';

test('publication diagnostics explicitly separate phases and never masquerade as arrival', () => {
  diagnosticLog.clear();
  recordInboxTiming({
    kind: 'publication',
    store: 'private-store',
    reason: 'read',
    preparation: 1,
    historyBindings: 2,
    subscribers: 3,
    channels: 10,
    conversations: 9,
    blockedChannels: 1,
    channelRevisions: 10,
    refreshRevisions: 10,
    stores: 2,
    listeners: 4,
  });
  const events = diagnosticLog.events();
  assert.equal(events.length, 3);
  assert.deepEqual(
    events.map((e) => [e.name, e.phase, e.ms]),
    [
      ['chat.publication', 'preparation', 1],
      ['chat.publication', 'history-bindings', 2],
      ['chat.publication', 'subscribers', 3],
    ],
  );
  for (const event of events) {
    assert.equal(event.scope, `store#${hashId('private-store')}`);
    assert.deepEqual(event.attrs, {
      reason: 'read',
      channels: 10,
      conversations: 9,
      blockedChannels: 1,
      channelRevisions: 10,
      refreshRevisions: 10,
      stores: 2,
      listeners: 4,
    });
  }
  recordInboxTiming({
    kind: 'arrival',
    store: 'private-store',
    milliseconds: 4,
  });
  assert.equal(diagnosticLog.events().at(-1)!.name, 'chat.arrival');
  diagnosticLog.clear();
});
