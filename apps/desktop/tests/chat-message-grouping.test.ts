import assert from 'node:assert/strict';
import test from 'node:test';
import { continuesMessageGroup } from '../src/chat/presentation';
import type { ChatMessage } from '../src/chat-contract';

const noon = new Date(2026, 8, 20, 12).valueOf();
function message(at: number, sender: string | null = 'alice'): ChatMessage {
  return {
    id: String(at),
    sequence: '1',
    sender,
    send_time: String(at),
    insert_time: String(at),
    content: { kind: 'text', text: 'Hello' },
  };
}

test('consecutive messages group by sender and the interval since the previous message', () => {
  assert.equal(continuesMessageGroup(undefined, message(noon)), false);
  assert.equal(continuesMessageGroup(message(noon), message(noon)), true);
  assert.equal(
    continuesMessageGroup(message(noon), message(noon + 300_000)),
    true,
  );
  assert.equal(
    continuesMessageGroup(message(noon + 300_000), message(noon + 600_000)),
    true,
  );
  assert.equal(
    continuesMessageGroup(message(noon), message(noon + 300_001)),
    false,
  );
  assert.equal(continuesMessageGroup(message(noon), message(noon - 1)), false);
  assert.equal(
    continuesMessageGroup(message(noon), message(noon, 'bob')),
    false,
  );
  assert.equal(
    continuesMessageGroup(message(noon, null), message(noon, null)),
    false,
  );
});

test('a new local day starts a fresh group even within five minutes', () => {
  const midnight = new Date(2026, 8, 21).valueOf();
  assert.equal(
    continuesMessageGroup(message(midnight - 1), message(midnight)),
    false,
  );
});
