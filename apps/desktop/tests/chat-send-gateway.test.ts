import assert from 'node:assert/strict';
import test from 'node:test';
import type {
  ChatAction,
  ChatOperation,
  ChatReply,
  ChatScope,
} from '../src/chat-contract';
import type { chatClient } from '../src/chat/client';
import {
  actionChannel,
  authorizer,
  ChatRequestGateway,
  coalesce,
} from '../src/chat/send/gateway';
import type { GatewayHost, TeamAccess } from '../src/chat/send/gateway';
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

type Send = (
  action: ChatAction,
  authorize: (phase: 'before' | 'after') => void,
) => Promise<ChatReply>;

function team(send: Send = () => Promise.reject(new Error('not used'))): Team {
  const client = {
    request: (
      action: ChatAction,
      _signal: unknown,
      authorize: (phase: 'before' | 'after') => void,
    ) => send(action, authorize),
    run: () => Promise.reject(new Error('not used')),
    dispose: () => {},
  } as unknown as ReturnType<typeof chatClient>;
  return newTeam({ storeId: 'team:eng', profile: 'acme', scope: null, client });
}

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

const scope = (actor: string): ChatScope => ({
  store: {
    profile: 'acme',
    account_alias: 'alice',
    team_alias: 'eng',
    team_id: 't'.repeat(32),
  },
  host: 'foks.example',
  actor,
});
const pending = (actor = 'alice'): ChatReply => ({
  scope: scope(actor),
  result: { kind: 'pending', operations: [] },
});

/** A gateway over a stub host that records what the gateway reports. */
function gateway() {
  const record = {
    applied: [] as ChatAction[],
    failed: [] as ChatAction[],
    handled: [] as unknown[],
    invalidated: [] as string[],
  };
  const host: GatewayHost = {
    ...access({
      current: (t, epoch) => epoch === undefined || t.epoch === epoch,
    }),
    inbox: {
      scope: () => undefined,
      handleError: (_storeId, error) => record.handled.push(error),
      invalidate: (storeId) => record.invalidated.push(storeId),
    },
    apply: (_team, _reply, action) => record.applied.push(action),
    failed: (_team, action) => record.failed.push(action),
  };
  return { gateway: new ChatRequestGateway(host), record };
}

test('the gateway adopts a reply scope and reconciles the reply', async () => {
  const { gateway: g, record } = gateway();
  const t = team(() => Promise.resolve(pending()));
  await g.request(t, { action: 'pending' }, 'background');
  assert.deepEqual(t.scope, scope('alice'));
  assert.deepEqual(record.applied, [{ action: 'pending' }]);
  assert.deepEqual(record.failed, []);
});

test('the gateway shares one run among identical coalescing requests', async () => {
  const { gateway: g } = gateway();
  let sent = 0;
  const t = team(() => {
    sent++;
    return Promise.resolve(pending());
  });
  await Promise.all([
    g.request(t, { action: 'pending' }, 'background'),
    g.request(t, { action: 'pending' }, 'foreground'),
  ]);
  assert.equal(sent, 1);
});

test('the gateway rejects a reply for another scope without applying it', async () => {
  const { gateway: g, record } = gateway();
  const t = team(() => Promise.resolve(pending('mallory')));
  t.scope = scope('alice');
  await assert.rejects(g.request(t, { action: 'pending' }, 'background'), {
    code: 'chat-integrity',
  });
  assert.deepEqual(t.scope, scope('alice'));
  assert.deepEqual(record.applied, []);
  assert.equal(record.handled.length, 1);
  assert.deepEqual(record.failed, [{ action: 'pending' }]);
});

test('an access-denied failure invalidates the team inbox', async () => {
  const { gateway: g, record } = gateway();
  const denied = {
    code: 'chat-access-denied',
    message: 'Your role no longer allows chat in this team.',
    fatal: false,
    retryable: false,
    ambiguous: false,
  };
  const t = team(async () => {
    throw denied;
  });
  await assert.rejects(g.request(t, submit, 'foreground'));
  assert.deepEqual(record.invalidated, ['team:eng']);
  assert.deepEqual(record.handled, [denied]);
  assert.deepEqual(record.failed, [submit]);

  // Other failures leave the inbox's own retry schedule in place.
  const other = gateway();
  const u = team(() => Promise.reject(new Error('offline')));
  await assert.rejects(other.gateway.request(u, submit, 'foreground'));
  assert.deepEqual(other.record.invalidated, []);
  assert.deepEqual(other.record.failed, [submit]);
});

test('a failure of a retired team is not reported', async () => {
  const { gateway: g, record } = gateway();
  let fail!: (error: unknown) => void;
  const t = team(
    () =>
      new Promise<ChatReply>((_resolve, reject) => {
        fail = reject;
      }),
  );
  const work = g.request(t, submit, 'foreground');
  retire(t);
  fail(new Error('offline'));
  await assert.rejects(work, /offline/);
  assert.deepEqual(record.handled, []);
  assert.deepEqual(record.invalidated, []);
  assert.deepEqual(record.failed, []);
});
