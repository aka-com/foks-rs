import assert from 'node:assert/strict';
import test from 'node:test';
import type { ChatOperation } from '../src/chat-contract';
import type { chatClient } from '../src/chat/client';
import { newOutgoing } from '../src/chat/send/outgoing';
import { newTeam, retire, type Team } from '../src/chat/send/team';

const CHANNEL = 'b'.repeat(32);
const op = (id: string, state: ChatOperation['state']): ChatOperation => ({
  id,
  channel: CHANNEL,
  kind: 'send-message',
  state,
  confirmation: null,
  rejection_code: null,
});

function team(disposed = { count: 0 }): Team {
  const client = {
    request: () => Promise.reject(new Error('not used')),
    run: () => Promise.reject(new Error('not used')),
    dispose: () => {
      disposed.count++;
    },
  } as unknown as ReturnType<typeof chatClient>;
  return newTeam({ storeId: 'team:eng', profile: 'acme', scope: null, client });
}

test('retiring a team discards its messages and everything it learned', () => {
  const disposed = { count: 0 };
  const t = team(disposed);
  const message = newOutgoing({
    channel: CHANNEL,
    submission: 's'.repeat(32),
    text: 'hello',
    createdAt: 0,
  });
  t.messages.push(message);
  t.operations.push({ ...op('a'.repeat(32), 'uncertain'), text: 'hello' });
  t.observations.set('m', {
    id: 'm',
    sequence: '1',
    sender: null,
    send_time: '0',
    insert_time: '0',
    content: { kind: 'text', text: 'hello' },
    channel: CHANNEL,
  });
  t.loaded.add(CHANNEL);
  t.loadErrors.set(CHANNEL, 'Load failed');
  t.checks.set(`status:${'a'.repeat(32)}`, { due: 10, failures: 2 });
  t.recovered = true;
  t.jobs = true;
  t.cleanupError = 'Cleanup failed';
  const messages = t.messages;
  const operations = t.operations;

  retire(t);

  assert.equal(t.epoch, 1);
  assert.equal(t.blocked, true);
  assert.equal(disposed.count, 1);
  // The message record a drive in flight holds loses its text.
  assert.equal(message.text, undefined);
  assert.deepEqual(t.messages, []);
  assert.deepEqual(t.operations, []);
  // The arrays are replaced, not truncated.
  assert.equal(messages.length, 1);
  assert.equal(operations.length, 1);
  assert.equal(t.observations.size, 0);
  assert.equal(t.loaded.size, 0);
  assert.equal(t.loadErrors.size, 0);
  assert.equal(t.checks.size, 0);
  assert.equal(t.recovered, false);
  assert.equal(t.jobs, false);
  assert.equal(t.cleanupError, '');
});
