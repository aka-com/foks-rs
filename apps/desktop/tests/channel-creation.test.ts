import assert from 'node:assert/strict';
import test from 'node:test';
import type { Bridge } from '../src/bridge';
import type {
  ChatAction,
  ChatOperation,
  ChatReply,
  ChatScope,
} from '../src/chat-contract';
import {
  ChannelCreationController,
  CHANNEL_CREATION_COMPLETION_LIMIT,
  CHANNEL_CREATION_RECORD_LIMIT,
} from '../src/chat/channel-creation';
import type { TeamInbox } from '../src/chat/inbox-service';
import { FIXTURE } from '../src/fixture';
import type { AgentSnapshot, TeamStore } from '../src/model';

const scope: ChatScope = {
  store: { profile: 'p', account_alias: 'a', team_alias: 't', team_id: 'team' },
  host: 'host',
  actor: 'actor',
};
const prepared: ChatOperation = {
  id: 'operation',
  channel: 'channel',
  kind: 'create-channel',
  state: 'prepared',
  receipt: null,
  rejection_code: null,
};
const confirmed: ChatOperation = {
  ...prepared,
  state: 'confirmed',
  receipt: { kind: 'channel-created' },
};
const input = { name: 'design', description: 'design discussion', admin: true };
const reply = (operation: ChatOperation): ChatReply => ({
  scope,
  result: { kind: 'operation', operation },
});
const tick = () => new Promise<void>((resolve) => setImmediate(resolve));

async function setup(
  respond: (action: ChatAction) => Promise<ChatReply>,
  pending: ChatOperation[] = [],
) {
  let snapshot: AgentSnapshot = {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server) => ({
      ...server,
      services: { ...server.services, chat: true },
      compatibility: { status: 'not-required' },
    })),
  };
  const store = snapshot.stores.find(
    (candidate): candidate is TeamStore =>
      candidate.kind === 'team' &&
      candidate.team_kind === 'named' &&
      candidate.active !== false,
  )!;
  let generation = 0;
  const actions: ChatAction[] = [];
  const blocked: string[] = [];
  const invalidated: string[] = [];
  const inbox = new Map<string, TeamInbox>([
    [
      store.id,
      {
        scope,
        state: 'ready',
        error: '',
        note: '',
        stale: false,
        revision: 1,
        channelRevisions: new Map(),
        blockedChannels: new Set(),
        data: {
          kind: 'inbox',
          channels: [],
          conversations: [],
          cursor: '0',
          head: '0',
          degraded: false,
          blocked_channels: [],
          read_retry_pending: false,
          previews_incomplete: false,
        },
      },
    ],
  ]);
  const bridge = {
    chat: async (_store: string, action: ChatAction) => {
      actions.push(action);
      if (action.action === 'pending')
        return { scope, result: { kind: 'pending', operations: pending } };
      return respond(action);
    },
    cancelChat: async () => {},
  } as unknown as Bridge;
  const controller = new ChannelCreationController(
    bridge,
    {
      getSnapshot: () => inbox,
      handleError: (id, cause) => {
        if ((cause as { code?: string }).code !== 'chat-integrity')
          return false;
        blocked.push(id);
        return true;
      },
      block: (id) => {
        blocked.push(id);
      },
      invalidate: (id) => {
        invalidated.push(id);
      },
    },
    () => ({
      snapshot,
      accessNow: () => 0,
      accessGenerations: new Map([[store.server, generation]]),
    }),
  );
  await controller.discover(store);
  return {
    controller,
    store,
    actions,
    blocked,
    invalidated,
    inbox,
    changeGeneration: () => {
      generation++;
    },
    denyAccess: () => {
      snapshot = { ...snapshot, agent: { state: 'bootstrap', step: 'unlock' } };
    },
  };
}

test('submitted creation survives detaching its observer and retains one identity until exact confirmation', async () => {
  let finish!: (reply: ChatReply) => void;
  const context = await setup(async (action) =>
    action.action === 'prepare-channel'
      ? reply(prepared)
      : new Promise((resolve) => {
          finish = resolve;
        }),
  );
  const { controller, store, actions } = context;
  const detach = controller.subscribe(() => {});
  const id = controller.submit(store, input);
  const published = controller.getSnapshot()[0];
  assert.equal(Object.isFrozen(published.input), true);
  assert.equal(Reflect.set(published.input!, 'name', 'changed'), false);
  assert.notEqual(published.store, store);
  assert.notEqual(published.scope, scope);
  assert.equal(controller.submit(store, { ...input, name: 'another' }), id);
  await tick();
  detach();
  assert.equal(controller.getSnapshot()[0].state, 'working');
  finish(reply(confirmed));
  await tick();
  assert.equal(controller.getSnapshot()[0].state, 'confirmed');
  assert.equal(controller.getSnapshot()[0].input, undefined);
  assert.equal(
    actions.filter((action) => action.action === 'prepare-channel').length,
    1,
  );
  assert.deepEqual(context.invalidated, [store.id]);
  controller.dispose();
});

test('uncertain is not success and checking it uses reconcile on the saved operation', async () => {
  const { controller, store, actions } = await setup(async (action) =>
    reply(
      action.action === 'prepare-channel'
        ? prepared
        : action.action === 'attempt'
          ? { ...prepared, state: 'uncertain' }
          : confirmed,
    ),
  );
  const id = controller.submit(store, input);
  await tick();
  assert.equal(controller.getSnapshot()[0].state, 'unresolved');
  assert.deepEqual(controller.getSnapshot()[0].input, input);
  controller.retry(id);
  await tick();
  assert.equal(controller.getSnapshot()[0].state, 'confirmed');
  assert.deepEqual(
    actions.map((action) => action.action),
    ['pending', 'prepare-channel', 'attempt', 'reconcile'],
  );
  controller.dispose();
});

test('a lost preparation reply retries the same submission and original fields', async () => {
  let first = true;
  const { controller, store, actions } = await setup(async (action) => {
    if (action.action === 'prepare-channel' && first) {
      first = false;
      throw {
        code: 'transport',
        message: 'Reply lost',
        ambiguous: true,
        fatal: false,
        retryable: false,
      };
    }
    return reply(action.action === 'prepare-channel' ? prepared : confirmed);
  });
  const id = controller.submit(store, input);
  await tick();
  assert.equal(controller.getSnapshot()[0].state, 'unresolved');
  controller.retry(id);
  await tick();
  const submissions = actions.filter(
    (action) => action.action === 'prepare-channel',
  );
  assert.equal(submissions.length, 2);
  assert.deepEqual(submissions[0], submissions[1]);
  assert.equal(controller.getSnapshot()[0].state, 'confirmed');
  controller.dispose();
});

test('fresh generation authorization rejects a late preparation without attempting it', async () => {
  let finish!: (reply: ChatReply) => void;
  const { controller, store, actions, changeGeneration } = await setup(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  controller.submit(store, input);
  await tick();
  changeGeneration();
  finish(reply(prepared));
  await tick();
  assert.equal(controller.getSnapshot()[0].state, 'unresolved');
  assert.equal(controller.getSnapshot()[0].operation, undefined);
  assert.equal(
    actions.some((action) => action.action === 'attempt'),
    false,
  );
  controller.dispose();
});

test('changed scope and mismatched confirmed operation never count as success', async () => {
  for (const altered of [
    { ...reply(prepared), scope: { ...scope, actor: 'other' } },
    reply({ ...confirmed, channel: 'other-channel' }),
    reply({ ...confirmed, id: 'other-operation' }),
  ]) {
    const { controller, store } = await setup(async (action) =>
      action.action === 'prepare-channel' && altered.scope.actor === scope.actor
        ? reply(prepared)
        : altered,
    );
    controller.submit(store, input);
    await tick();
    assert.equal(controller.getSnapshot()[0].state, 'review');
    controller.dispose();
  }
});

test('ledger discovery never auto-attempts an unassociated prepared operation', async () => {
  const { controller, store, actions } = await setup(
    async () => reply(prepared),
    [prepared],
  );
  const saved = controller.getSnapshot()[0];
  assert.equal(saved.input, undefined);
  assert.equal(controller.submit(store, input), saved.id);
  controller.retry(saved.id);
  await tick();
  assert.deepEqual(
    actions.map((action) => action.action),
    ['pending', 'reconcile'],
  );
  assert.equal(controller.getSnapshot()[0].state, 'unresolved');
  controller.dispose();
});

test('stale preparations require review or cancellation, not an endless retry loop', async () => {
  const { controller, store, actions } = await setup(async (action) => {
    if (action.action === 'prepare-channel') return reply(prepared);
    if (action.action === 'cancel')
      return reply({ ...prepared, state: 'cancelled' });
    throw {
      code: 'chat-stale-key',
      message: 'Keys changed',
      ambiguous: false,
      retryable: false,
      fatal: false,
    };
  });
  const id = controller.submit(store, input);
  await tick();
  assert.equal(controller.getSnapshot()[0].state, 'review');
  controller.retry(id);
  await tick();
  assert.equal(
    actions.filter((action) => action.action === 'attempt').length,
    1,
  );
  assert.equal(controller.submit(store, input), id);
  controller.cancel(id);
  await tick();
  assert.equal(controller.getSnapshot()[0].state, 'cancelled');
  controller.dispose();
});

test('a definite rejection requires explicit review and never tries to cancel a terminal operation', async () => {
  const { controller, store, actions } = await setup(async (action) =>
    reply(
      action.action === 'prepare-channel'
        ? prepared
        : { ...prepared, state: 'rejected', rejection_code: 42 },
    ),
  );
  const id = controller.submit(store, input);
  await tick();
  assert.equal(controller.getSnapshot()[0].state, 'review');
  assert.equal(controller.submit(store, input), id);
  controller.cancel(id);
  await tick();
  assert.equal(
    actions.some((action) => action.action === 'cancel'),
    false,
  );
  controller.review(id);
  assert.deepEqual(controller.getSnapshot(), []);
  controller.dispose();
});

test('disposal clears raw form data and ignores a late response', async () => {
  let finish!: (reply: ChatReply) => void;
  const { controller, store, actions } = await setup(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  controller.submit(store, input);
  await tick();
  controller.dispose();
  finish(reply(prepared));
  await tick();
  assert.deepEqual(controller.getSnapshot(), []);
  assert.equal(
    actions.some((action) => action.action === 'attempt'),
    false,
  );
});

test('queued work checks current access before contacting the bridge', async () => {
  const { controller, store, actions, denyAccess } = await setup(async () =>
    reply(prepared),
  );
  controller.submit(store, input);
  denyAccess();
  await tick();
  assert.deepEqual(
    actions.map((action) => action.action),
    ['pending'],
  );
  assert.equal(controller.getSnapshot()[0].state, 'cancelled');
  controller.dispose();
});

test('a late confirmed reply after access loss stays unresolved', async () => {
  let finish!: (value: ChatReply) => void;
  const { controller, store, denyAccess } = await setup(async (action) =>
    action.action === 'prepare-channel'
      ? reply(prepared)
      : new Promise((resolve) => {
          finish = resolve;
        }),
  );
  controller.submit(store, input);
  await tick();
  denyAccess();
  finish(reply(confirmed));
  await tick();
  assert.equal(controller.getSnapshot()[0].state, 'unresolved');
  assert.equal(controller.getSnapshot()[0].operation?.id, prepared.id);
  controller.dispose();
});

test('a name taken since preparation requires review rather than another attempt', async () => {
  const { controller, store, inbox, actions } = await setup(async (action) => {
    if (action.action === 'attempt')
      throw {
        code: 'transport',
        message: 'Lost reply',
        ambiguous: true,
        fatal: false,
        retryable: false,
      };
    return reply(prepared);
  });
  const id = controller.submit(store, input);
  await tick();
  const entry = inbox.get(store.id)!;
  entry.data!.channels = [
    {
      id: 'another-channel',
      name: input.name,
      description: null,
      admin: false,
      readable: true,
      writable: true,
      read_role: 'member',
      write_role: 'member',
    },
  ];
  controller.retry(id);
  await tick();
  assert.equal(controller.getSnapshot()[0].state, 'review');
  assert.equal(
    actions.filter((action) => action.action === 'attempt').length,
    1,
  );
  assert.equal(controller.submit(store, { ...input, name: 'different' }), id);
  controller.dispose();
});

test('a changed access generation invalidates the pending-ledger prerequisite', async () => {
  const { controller, store, changeGeneration, actions } = await setup(
    async () => reply(prepared),
  );
  changeGeneration();
  assert.equal(controller.readyFor(store.id), false);
  assert.throws(
    () => controller.submit(store, input),
    /Check saved channel creations/,
  );
  await controller.discover(store);
  assert.equal(controller.readyFor(store.id), true);
  assert.deepEqual(
    actions.map((action) => action.action),
    ['pending', 'pending'],
  );
  controller.dispose();
});

test('the native chat-reprepare-required error stops retries until review or cancellation', async () => {
  const { controller, store, actions } = await setup(async (action) => {
    if (action.action === 'prepare-channel') return reply(prepared);
    if (action.action === 'cancel')
      return reply({ ...prepared, state: 'cancelled' });
    throw {
      code: 'chat-reprepare-required',
      message: 'Prepare again with current keys',
      ambiguous: false,
      retryable: false,
      fatal: false,
    };
  });
  const id = controller.submit(store, input);
  await tick();
  assert.equal(controller.getSnapshot()[0].state, 'review');
  assert.deepEqual(controller.getSnapshot()[0].input, input);
  controller.retry(id);
  controller.check(id);
  await tick();
  assert.equal(
    actions.filter((action) => action.action === 'attempt').length,
    1,
  );
  assert.equal(controller.submit(store, { ...input, name: 'different' }), id);
  controller.cancel(id);
  await tick();
  assert.equal(controller.getSnapshot()[0].state, 'cancelled');
  controller.acknowledge(id);
  assert.deepEqual(controller.getSnapshot(), []);
  controller.dispose();
});

test('Check again never delivers a prepared operation; explicit retry is required', async () => {
  let attempts = 0;
  const { controller, store, actions } = await setup(async (action) => {
    if (action.action === 'attempt') {
      attempts++;
      if (attempts === 1) return reply({ ...prepared, state: 'uncertain' });
      return reply(confirmed);
    }
    return reply(prepared);
  });
  const id = controller.submit(store, input);
  await tick();
  controller.check(id);
  await tick();
  assert.equal(controller.getSnapshot()[0].operation?.state, 'prepared');
  assert.equal(controller.getSnapshot()[0].state, 'unresolved');
  assert.equal(attempts, 1);
  controller.retry(id);
  await tick();
  assert.equal(attempts, 2);
  assert.deepEqual(
    actions.map((action) => action.action),
    [
      'pending',
      'prepare-channel',
      'attempt',
      'reconcile',
      'reconcile',
      'attempt',
    ],
  );
  controller.dispose();
});

test('controller snapshots isolate every identity and operation object from callers', async () => {
  const pending = { ...prepared };
  const { controller, store, actions } = await setup(
    async () => reply(prepared),
    [pending],
  );
  const snapshot = controller.getSnapshot();
  const saved = snapshot[0];
  assert.equal(Object.isFrozen(snapshot), true);
  for (const [object, key] of [
    [saved, 'id'],
    [saved.store, 'id'],
    [saved.scope, 'actor'],
    [saved.scope.store, 'account_alias'],
    [saved.operation!, 'channel'],
  ] as const) {
    assert.equal(Reflect.set(object, key, 'changed'), false);
  }
  pending.channel = 'changed-after-discovery';
  assert.equal(
    controller.getSnapshot()[0].operation?.channel,
    prepared.channel,
  );
  controller.check(saved.id);
  await tick();
  assert.equal(controller.getSnapshot()[0].state, 'unresolved');
  assert.deepEqual(actions.at(-1), {
    action: 'reconcile',
    operation: prepared.id,
  });
  assert.equal(controller.getSnapshot()[0].store.id, store.id);
  controller.dispose();
});

test('successful history is bounded and acknowledged terminal records are removed', async () => {
  let sequence = 0;
  let current = prepared;
  const { controller, store } = await setup(async (action) => {
    if (action.action === 'prepare-channel') {
      current = {
        ...prepared,
        id: `operation-${++sequence}`,
        channel: `channel-${sequence}`,
      };
      return reply(current);
    }
    return reply({
      ...current,
      state: 'confirmed',
      receipt: { kind: 'channel-created' },
    });
  });
  for (let index = 0; index < CHANNEL_CREATION_COMPLETION_LIMIT + 8; index++) {
    controller.submit(store, input);
    await tick();
  }
  const completed = controller.getSnapshot();
  assert.equal(completed.length, CHANNEL_CREATION_COMPLETION_LIMIT);
  assert.equal(
    completed.every((record) => record.input === undefined),
    true,
  );
  for (const record of completed) controller.acknowledge(record.id);
  assert.deepEqual(controller.getSnapshot(), []);
  controller.dispose();
});

test('pending-ledger overflow stays bounded without evicting ambiguous records or admitting duplicates', async () => {
  const pending = Array.from(
    { length: CHANNEL_CREATION_RECORD_LIMIT + 1 },
    (_, index) => ({
      ...prepared,
      id: `saved-${index}`,
      channel: `channel-${index}`,
    }),
  );
  const { controller, store } = await setup(
    async () => reply(prepared),
    pending,
  );
  assert.equal(controller.getSnapshot().length, CHANNEL_CREATION_RECORD_LIMIT);
  assert.equal(controller.readyFor(store.id), false);
  const first = controller.getSnapshot()[0];
  controller.acknowledge(first.id);
  assert.equal(controller.getSnapshot().length, CHANNEL_CREATION_RECORD_LIMIT);
  assert.equal(controller.submit(store, input), first.id);
  controller.dispose();
});

test('completion destinations require fresh access and cannot redirect after acknowledgment', async () => {
  const { controller, store, denyAccess } = await setup(async (action) =>
    reply(action.action === 'prepare-channel' ? prepared : confirmed),
  );
  const id = controller.submit(store, input);
  await tick();
  assert.deepEqual(controller.completedDestination(id), {
    ref: store.id,
    channel: prepared.channel,
  });
  denyAccess();
  assert.throws(
    () => controller.completedDestination(id),
    (error: unknown) =>
      (error as { code: string }).code === 'chat-access-denied',
  );
  controller.acknowledge(id);
  assert.equal(controller.completedDestination(id), null);
  controller.dispose();
});

for (const name of ['general', 'GeNeRaL', '  General  ']) {
  test(`${JSON.stringify(name)} prepares the protocol’s unnamed general channel`, async () => {
    const { controller, store, actions } = await setup(async (action) =>
      reply(action.action === 'prepare-channel' ? prepared : confirmed),
    );
    try {
      const id = controller.submit(store, { ...input, name });
      assert.equal(
        controller.getSnapshot().find((record) => record.id === id)?.input
          ?.name,
        '',
      );
      await tick();
      const preparation = actions.find(
        (action) => action.action === 'prepare-channel',
      );
      assert.ok(preparation?.action === 'prepare-channel');
      assert.equal(preparation.name, '');
    } finally {
      controller.dispose();
    }
  });
}

test('protocol name and description validation applies before durable preparation', async () => {
  const { controller, store, actions } = await setup(async () =>
    reply(prepared),
  );
  assert.throws(
    () => controller.submit(store, { ...input, name: 'bad name' }),
    /cannot contain spaces/,
  );
  assert.throws(
    () => controller.submit(store, { ...input, description: 'a' }),
    /Descriptions must/,
  );
  assert.deepEqual(
    actions.map((action) => action.action),
    ['pending'],
  );
  controller.dispose();
});
