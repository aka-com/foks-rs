import assert from 'node:assert/strict';
import test from 'node:test';
import type { ChatAction, ChatOperation } from '../src/chat-contract';
import type { chatClient } from '../src/chat/client';
import { actionChannel, authorizer, coalesce } from '../src/chat/send/gateway';
import type { TeamAccess } from '../src/chat/send/gateway';
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

test('coalesce shares one run per key until it settles', async () => {
  const inflight = new Map<string, Promise<number>>();
  let runs = 0;
  let release!: (value: number) => void;
  const start = () => {
    runs++;
    return new Promise<number>((resolve) => {
      release = resolve;
    });
  };
  const first = coalesce(inflight, 'k', start);
  const second = coalesce(inflight, 'k', start);
  assert.equal(first, second);
  assert.equal(runs, 1);
  release(7);
  assert.equal(await first, 7);
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.equal(inflight.size, 0);
  void coalesce(inflight, 'k', start);
  assert.equal(runs, 2);
  release(0);
});

test('coalesce removes a rejected run without an unhandled rejection', async () => {
  const inflight = new Map<string, Promise<void>>();
  const work = coalesce(inflight, 'k', () => Promise.reject(new Error('x')));
  await assert.rejects(work, /x/);
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.equal(inflight.size, 0);
});

test('an action addresses its channel directly or through its operation', () => {
  const t = team();
  t.operations.push(op('a'.repeat(32), 'prepared'));
  assert.equal(
    actionChannel(t, { action: 'history', channel: 'c', before: null }),
    'c',
  );
  assert.equal(
    actionChannel(t, { action: 'attempt', operation: 'a'.repeat(32) }),
    CHANNEL,
  );
  assert.equal(
    actionChannel(t, { action: 'attempt', operation: 'z'.repeat(32) }),
    undefined,
  );
  assert.equal(actionChannel(t, { action: 'pending' }), undefined);
});

function access(overrides: Partial<TeamAccess> = {}): TeamAccess {
  return {
    current: () => true,
    available: () => true,
    readable: () => true,
    writable: () => true,
    channelBlocked: () => false,
    generation: () => 0,
    agentReady: () => true,
    ...overrides,
  };
}
const submit: ChatAction = {
  action: 'submit-message',
  submission: 's'.repeat(32),
  channel: CHANNEL,
  text: 'hello',
};
const code = (fn: () => void) => {
  try {
    fn();
  } catch (error) {
    return (error as { code: string }).code;
  }
  return 'none';
};

test('a request is authorized against the access it was started under', () => {
  const t = team();
  const check = (
    a: TeamAccess,
    action: ChatAction,
    phase: 'before' | 'after',
  ) => code(() => authorizer(a, t, action, 0, 0, CHANNEL)(phase));
  assert.equal(check(access(), submit, 'before'), 'none');
  assert.equal(
    check(access({ current: () => false }), submit, 'before'),
    'cancelled',
  );
  assert.equal(
    check(access({ available: () => false }), submit, 'after'),
    'access-changed',
  );
  // A lock or rekey since the request started changes the access generation.
  assert.equal(
    check(access({ generation: () => 1 }), submit, 'after'),
    'access-changed',
  );
  // Only a submit checks the channel's write access, and only before it is sent.
  assert.equal(
    check(access({ writable: () => false }), submit, 'before'),
    'access-changed',
  );
  assert.equal(
    check(access({ writable: () => false }), submit, 'after'),
    'none',
  );
  assert.equal(
    check(
      access({ writable: () => false }),
      { action: 'attempt', operation: 'a'.repeat(32) },
      'before',
    ),
    'none',
  );
  assert.equal(
    check(access({ agentReady: () => false }), submit, 'before'),
    'cancelled',
  );
  assert.equal(
    check(access({ channelBlocked: () => true }), submit, 'before'),
    'chat-channel-integrity',
  );
  // A local action reads the agent's own store, so chat availability, the
  // access generation and a blocked channel do not stop it.
  const status: ChatAction = { action: 'status', operation: 'a'.repeat(32) };
  assert.equal(
    check(
      access({
        available: () => false,
        generation: () => 1,
        channelBlocked: () => true,
      }),
      status,
      'before',
    ),
    'none',
  );
});
