import assert from 'node:assert/strict';
import test from 'node:test';
import { PROTOCOL_CAPABILITIES } from '../src/model/types';
import { cancelled } from '../src/chat/client';
import { ChatInboxService } from '../src/chat/inbox-service';
import type { ChatClock, ChatInboxTiming } from '../src/chat/inbox-service';
import type { Bridge } from '../src/bridge';
import type { ChatAction, ChatReply } from '../src/chat-contract';
import type { TeamStore, AgentSnapshot } from '../src/model';
import { FIXTURE } from '../src/fixture';

function chatSnapshot(stores: TeamStore[]): AgentSnapshot {
  const template = FIXTURE.servers[0];
  assert.ok(template);
  return {
    ...FIXTURE,
    stores,
    storeInventory: stores.map((store) => ({
      store: store.id,
      status: 'available',
      restrictions: [],
    })),
    servers: [
      {
        ...template,
        id: 'p',
        services: { chat: true },
        compatibility: { status: 'not-required' },
      },
    ],
  };
}
class Clock implements ChatClock {
  time = 0;
  id = 0;
  tasks = new Map<number, { due: number; fn: () => void }>();
  now = () => this.time;
  random = () => 0;
  later = (fn: () => void, delay: number) => {
    const id = ++this.id;
    this.tasks.set(id, { due: this.time + delay, fn });
    return id;
  };
  cancel = (id: unknown) => {
    this.tasks.delete(id as number);
  };
  async advance(ms: number) {
    const end = this.time + ms;
    for (;;) {
      const next = [...this.tasks].sort((a, b) => a[1].due - b[1].due)[0];
      if (!next || next[1].due > end) break;
      this.time = next[1].due;
      this.tasks.delete(next[0]);
      next[1].fn();
      for (let i = 0; i < 30; i++) await Promise.resolve();
    }
    this.time = end;
  }
}
function fixture(count = 2) {
  const stores: TeamStore[] = Array.from({ length: count }, (_, i) => ({
    id: `t${i}`,
    kind: 'team',
    name: `Team ${i}`,
    alias: `t${i}`,
    server: 'p',
    account: 'me',
    team_kind: 'named',
    team_id_hex: String(i),
    active: true,
  }));
  const snapshot = chatSnapshot(stores);
  const clock = new Clock();
  const waits = new Map<
    string,
    {
      resolve: (reply: ChatReply) => void;
      reject: (cause: unknown) => void;
      store: TeamStore;
      action: Extract<ChatAction, { action: 'poll-inbox' }>;
    }
  >();
  const syncs: string[] = [];
  const polls: string[] = [];
  // What every team's inbox reply states. `position` is each channel's newest
  // sequence, which is what `lastPosition` derives from an unread count.
  const inbox = { channels: [] as string[], degraded: false, position: 0 };
  const channelRow = (id: string) => ({
    id,
    name: id,
    description: null,
    admin: false,
    readable: true,
    writable: true,
    read_role: 'Member (0)',
    write_role: 'Member (0)',
  });
  const fail = new Set<string>();
  const denied = new Set<string>();
  const unrefreshed = new Set<string>();
  let head = '1';
  const scope = (store: TeamStore) => ({
    store: {
      profile: 'p',
      account_alias: 'me',
      team_alias: store.alias,
      team_id: store.team_id_hex,
    },
    host: 'host',
    actor: 'actor',
  });
  const bridge = {
    chat: async (
      id: string,
      action: ChatAction,
      view: string,
    ): Promise<ChatReply> => {
      const store = stores.find((s) => s.id === id)!;
      if (action.action === 'poll-inbox') {
        polls.push(action.since);
        assert.equal(waits.size, 0, 'one waiter per account');
        return new Promise((resolve, reject) =>
          waits.set(view, { resolve, reject, store, action }),
        );
      }
      assert.equal(action.action, 'sync-inbox');
      syncs.push(id);
      if (denied.has(id))
        throw {
          code: 'chat-access-denied',
          message: 'access denied',
          retryable: false,
          fatal: false,
          ambiguous: false,
        };
      if (fail.has(id))
        throw {
          code: 'io',
          message: 'offline',
          retryable: true,
          fatal: false,
          ambiguous: false,
        };
      if (unrefreshed.has(id))
        throw {
          code: 'catalog-required',
          message:
            'The vault for p has not been refreshed. Refresh and try again.',
          retryable: true,
          fatal: false,
          ambiguous: false,
        };
      return {
        scope: scope(store),
        result: {
          kind: 'inbox',
          channels: inbox.channels.map(channelRow),
          conversations: inbox.channels.map((id) => ({
            channel: channelRow(id),
            inbox_version: '1',
            read_through: '0',
            pending_read: null,
            unread: String(inbox.position),
            hidden: false,
            muted: false,
            preview: null,
          })),
          cursor: head,
          head,
          degraded: inbox.degraded,
          read_retry_pending: false,
          previews_incomplete: false,
          blocked_channels: [],
        },
      };
    },
    cancelChat: async (view: string) => {
      const wait = waits.get(view);
      waits.delete(view);
      wait?.reject({
        code: 'cancelled',
        message: 'closed',
        fatal: false,
        retryable: false,
        ambiguous: false,
      });
    },
  } as unknown as Bridge;
  const service = new ChatInboxService(bridge, clock);
  const bump = (value: string) => {
    head = value;
    for (const [view, wait] of waits) {
      waits.delete(view);
      wait.resolve({
        scope: scope(wait.store),
        result: {
          kind: 'poll',
          bumped: BigInt(value) > BigInt(wait.action.since),
          inbox_version: value,
        },
      });
    }
  };
  service.updateStores(snapshot);
  service.start();
  return {
    service,
    bridge,
    clock,
    inbox,
    syncs,
    polls,
    fail,
    denied,
    unrefreshed,
    bump,
    waits,
    snapshot,
  };
}
test('shell synchronizes all teams with one poll and cancels while Chat is absent', async () => {
  const f = fixture();
  await f.clock.advance(500);
  assert.deepEqual(new Set(f.syncs), new Set(['t0', 't1']));
  assert.equal(f.waits.size, 1);
  f.bump('2');
  await f.clock.advance(1000);
  assert.equal(f.service.getSnapshot().get('t1')?.data?.head, '2');
  f.service.stop();
  await f.clock.advance(1000);
  assert.equal(f.waits.size, 0);
  assert.equal(f.service.getSnapshot().size, 0);
  assert.equal(f.clock.tasks.size, 0);
});
for (const change of [
  'stop',
  'generation',
  'host',
  'probe',
  'removed',
] as const) {
  test(`inbox lifecycle clears retained histories after ${change}`, async () => {
    const f = fixture(1);
    try {
      await f.clock.advance(500);
      const entry = f.service.getSnapshot().get('t0')!;
      f.service.histories.update(
        't0',
        {
          ...entry,
          data: {
            ...entry.data!,
            channels: [
              {
                id: 'a',
                name: 'a',
                description: null,
                admin: false,
                readable: true,
                writable: true,
                read_role: 'Member (0)',
                write_role: 'Member (0)',
              },
            ],
          },
        },
        0,
      );
      const binding = f.service.histories.binding('t0', 'a', 0)!;
      f.service.histories.accept(
        binding,
        {
          kind: 'history',
          channel: 'a',
          before: null,
          messages: [],
          missing_predecessors: [],
        },
        null,
      );
      assert.ok(f.service.histories.get(binding));
      if (change === 'stop') f.service.stop();
      else {
        const snapshot = structuredClone(f.snapshot);
        if (change === 'host') snapshot.servers[0].host_id = 'other-host';
        if (change === 'probe')
          snapshot.servers[0].configuredProbe = 'other-probe';
        if (change === 'removed') snapshot.stores = [];
        f.service.updateStores(
          snapshot,
          {},
          new Map([['p', change === 'generation' ? 1 : 0]]),
        );
      }
      assert.equal(f.service.histories.get(binding), null);
    } finally {
      f.service.stop();
    }
  });
}

test('failed team retries independently and observed poll head advances before sync succeeds', async () => {
  const f = fixture();
  f.fail.add('t1');
  await f.clock.advance(500);
  f.bump('3');
  await f.clock.advance(1000);
  assert.equal(f.polls.at(-1), '3');
  assert.equal(f.service.getSnapshot().get('t1')?.stale, true);
  f.fail.clear();
  await f.clock.advance(3000);
  assert.equal(f.service.getSnapshot().get('t1')?.data?.head, '3');
  f.service.stop();
});
test('removing the canonical team hands off without leaving a second waiter', async () => {
  const f = fixture();
  await f.clock.advance(500);
  f.service.updateStores({ ...f.snapshot, stores: f.snapshot.stores.slice(1) });
  await f.clock.advance(1000);
  assert.equal(f.waits.size, 1);
  assert.equal(f.service.getSnapshot().has('t0'), false);
  f.service.stop();
});

test('lower head revalidates instead of continuing at the old poll cursor', async () => {
  const f = fixture();
  await f.clock.advance(500);
  f.bump('8');
  await f.clock.advance(1000);
  f.bump('2');
  await f.clock.advance(1000);
  assert.equal(f.polls.at(-1), '2');
  assert.equal(f.service.getSnapshot().get('t1')?.data?.head, '2');
  f.service.stop();
});
test('blocked accounts clear projections and require a fresh lifetime', async () => {
  const f = fixture();
  await f.clock.advance(500);
  f.service.block('t0', 'Invalid inbox identity.');
  await f.clock.advance(1000);
  assert.equal(f.waits.size, 0);
  assert.equal(f.service.getSnapshot().get('t1')?.data, undefined);
  const count = f.syncs.length;
  f.service.invalidate('t1');
  await f.clock.advance(30000);
  assert.equal(f.syncs.length, count);
  f.service.stop();
  f.service.updateStores(f.snapshot);
  f.service.start();
  await f.clock.advance(500);
  assert.equal(f.waits.size, 1);
  f.service.stop();
});

test('unknown inbox errors do not quarantine sibling teams and can be refreshed', async () => {
  const f = fixture();
  const chat = f.bridge.chat.bind(f.bridge);
  let failed = true;
  f.bridge.chat = async (id, action, view) => {
    if (failed && id === 't0' && action.action === 'sync-inbox')
      throw new Error('Projection failed.');
    return chat(id, action, view);
  };
  try {
    await f.clock.advance(500);
    assert.notEqual(f.service.getSnapshot().get('t0')?.state, 'blocked');
    assert.equal(f.service.getSnapshot().get('t1')?.state, 'ready');
    failed = false;
    f.service.invalidate('t0');
    await f.clock.advance(500);
    assert.equal(f.service.getSnapshot().get('t0')?.state, 'ready');
  } finally {
    f.service.stop();
  }
});

test('agent loss clears authorization but fresh synchronization can recover the same service', async () => {
  const f = fixture();
  try {
    await f.clock.advance(500);
    const wait = f.waits.entries().next().value!;
    f.waits.delete(wait[0]);
    wait[1].reject({
      code: 'agent-lost',
      message: 'Disconnected.',
      retryable: true,
      fatal: true,
      ambiguous: false,
    });
    for (let i = 0; i < 30; i++) await Promise.resolve();
    for (const id of ['t0', 't1']) {
      assert.equal(f.service.getSnapshot().get(id)?.state, 'unavailable');
      assert.equal(f.service.getSnapshot().get(id)?.data, undefined);
    }
    await f.clock.advance(2_000);
    for (const id of ['t0', 't1'])
      assert.equal(f.service.getSnapshot().get(id)?.state, 'ready');
  } finally {
    f.service.stop();
  }
});

test('quarantine is scoped and cannot be cleared by ordinary invalidation', () => {
  const f = fixture(3);
  try {
    f.service.stop();
    const stores = f.snapshot.stores.map((store) =>
      store.id === 't2' ? { ...store, account: 'other' } : store,
    );
    f.service.updateStores({ ...f.snapshot, stores });
    f.service.handleError('t0', {
      code: 'chat-integrity',
      message: 'Wrong actor.',
      retryable: false,
      fatal: true,
      ambiguous: false,
    });
    assert.equal(f.service.getSnapshot().get('t0')?.state, 'blocked');
    assert.equal(f.service.getSnapshot().get('t1')?.state, 'blocked');
    assert.equal(f.service.getSnapshot().get('t2')?.state, 'loading');
    assert.equal(
      f.service.getSnapshot().get('t0')?.failure?.code,
      'chat-integrity',
    );
    f.service.invalidate('t0');
    assert.equal(f.service.getSnapshot().get('t0')?.state, 'blocked');
  } finally {
    f.service.stop();
  }
});

test('an unscoped channel integrity failure quarantines only the affected team', async () => {
  const f = fixture();
  try {
    await f.clock.advance(500);
    f.service.handleError('t0', {
      code: 'chat-channel-integrity',
      message: 'Invalid channel.',
      retryable: false,
      fatal: true,
      ambiguous: false,
    });
    f.service.invalidate('t0');
    await f.clock.advance(1_000);
    assert.equal(f.service.getSnapshot().get('t0')?.state, 'blocked');
    assert.equal(f.service.getSnapshot().get('t1')?.state, 'ready');
  } finally {
    f.service.stop();
  }
});

test('late inbox replies cannot restore authorization retired by a disconnect', async () => {
  const f = fixture();
  try {
    await f.clock.advance(500);
    const revision = f.service.getSnapshot().get('t0')!.authorizationRevision!;
    const chat = f.bridge.chat.bind(f.bridge);
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    let held = true;
    f.bridge.chat = async (id, action, view) => {
      const reply = await chat(id, action, view);
      if (id === 't0' && action.action === 'sync-inbox' && held) await gate;
      return reply;
    };
    f.service.invalidate('t0');
    await f.clock.advance(0);
    f.service.handleError('t0', {
      code: 'agent-lost',
      message: 'Disconnected.',
      retryable: true,
      fatal: true,
      ambiguous: false,
    });
    held = false;
    release();
    for (let i = 0; i < 30; i++) await Promise.resolve();
    assert.equal(f.service.getSnapshot().get('t0')?.state, 'unavailable');
    assert.equal(
      f.service.getSnapshot().get('t0')?.authorizationRevision,
      revision,
    );
    await f.clock.advance(2_000);
    assert.equal(f.service.getSnapshot().get('t0')?.state, 'ready');
    assert.ok(
      f.service.getSnapshot().get('t0')!.authorizationRevision! > revision,
    );
  } finally {
    f.service.stop();
  }
});

test('finite admission serves excess accounts and releases more than the view limit over time', async () => {
  const clock = new Clock();
  const active = new Map<string, () => void>();
  const served = new Set<string>();
  let registrations = 0;
  let peak = 0;
  const stores: TeamStore[] = Array.from({ length: 6 }, (_, i) => ({
    id: `t${i}`,
    name: `Team ${i}`,
    kind: 'team',
    team_kind: 'named',
    active: true,
    server: 'p',
    account: `a${i}`,
    alias: `team${i}`,
    team_id_hex: `${i}`,
  }));
  const snapshot = chatSnapshot(stores);
  const bridge = {
    chat: async (
      id: string,
      action: ChatAction,
      view: string,
    ): Promise<ChatReply> => {
      const store = snapshot.stores.find((s) => s.id === id) as TeamStore;
      registrations++;
      active.set(view, () => {});
      peak = Math.max(peak, active.size);
      const scope = {
        store: {
          profile: 'p',
          account_alias: store.account,
          team_alias: store.alias,
          team_id: store.team_id_hex,
        },
        host: 'h',
        actor: store.account,
      };
      if (action.action === 'poll-inbox') {
        served.add(store.account);
        return new Promise((resolve, reject) => {
          const timer = clock.later(
            () =>
              resolve({
                scope,
                result: { kind: 'poll', bumped: false, inbox_version: '0' },
              }),
            25000,
          );
          active.set(view, () => {
            clock.cancel(timer);
            reject(cancelled());
          });
        });
      }
      return {
        scope,
        result: {
          kind: 'inbox',
          channels: [],
          conversations: [],
          cursor: '0',
          head: '0',
          degraded: false,
          read_retry_pending: false,
          previews_incomplete: false,
          blocked_channels: [],
        },
      };
    },
    cancelChat: async (view: string) => {
      active.get(view)?.();
      active.delete(view);
    },
  } as unknown as Bridge;
  const service = new ChatInboxService(bridge, clock, 2);
  service.updateStores(snapshot);
  service.start();
  await clock.advance(600000);
  assert.equal(served.size, 6);
  // Polls dominate this: each account long-polls for twenty-five seconds and
  // re-polls, while an idle team resynchronizes in minutes.
  assert.ok(registrations > 32, `registrations ${registrations}`);
  assert.ok(peak <= 3, `peak registrations ${peak}`);
  service.stop();
  await clock.advance(1000);
  assert.equal(active.size, 0);
});

test('eligibility follows the service clock, not the wall clock', async () => {
  const f = fixture();
  await f.clock.advance(500);
  assert.deepEqual(new Set(f.syncs), new Set(['t0', 't1']));
  // The server's check-in expires two seconds into the service's clock, which
  // stands at half a second. Nothing the wall clock says takes a team away.
  const expiresAt = 2;
  const lapsing = {
    ...f.snapshot,
    servers: f.snapshot.servers.map((server) => ({
      ...server,
      compatibility: {
        status: 'required' as const,
        capabilities: PROTOCOL_CAPABILITIES,
        expiresAt,
      },
    })),
  };
  f.service.updateStores(lapsing);
  await f.clock.advance(100);
  assert.deepEqual(
    [...f.service.getSnapshot().keys()].sort(),
    ['t0', 't1'],
    'a check-in still current on the service clock keeps both teams',
  );
  // Past the expiry on that same clock the teams are no longer kept: the Chat
  // tab decides reachability on the shell's clock and would otherwise wait for
  // entries that never arrive.
  await f.clock.advance(3000);
  f.service.updateStores(lapsing);
  assert.equal(f.service.getSnapshot().size, 0);
  f.service.stop();
});

test('access-denied canonical team hands polling to an accessible sibling', async () => {
  const f = fixture();
  f.denied.add('t0');
  await f.clock.advance(1500);
  assert.equal(f.service.getSnapshot().get('t0')?.state, 'unavailable');
  assert.equal(f.waits.size, 1);
  assert.equal([...f.waits.values()][0].store.id, 't1');
  f.bump('3');
  await f.clock.advance(1000);
  assert.equal(f.service.getSnapshot().get('t1')?.data?.head, '3');
  f.service.stop();
});

test('channel quarantine survives late sync and soft reset without blocking sibling teams', async () => {
  const f = fixture();
  const original = f.bridge.chat.bind(f.bridge);
  const id = 'ab'.repeat(16);
  let hold = false;
  let release: (() => void) | undefined;
  const exclusions: string[][] = [];
  f.bridge.chat = async (store, action, view) => {
    const reply = await original(store, action, view);
    if (
      store === 't0' &&
      action.action === 'sync-inbox' &&
      reply.result.kind === 'inbox'
    ) {
      exclusions.push(action.blocked_channels ?? []);
      const channel = {
        id,
        name: 'general',
        description: null,
        admin: false,
        readable: true,
        writable: true,
        read_role: 'Member (0)',
        write_role: 'Member (0)',
      };
      reply.result.channels = [channel];
      reply.result.conversations = [
        {
          channel,
          inbox_version: '1',
          read_through: '0',
          pending_read: null,
          unread: '1',
          hidden: false,
          muted: false,
          preview: {
            sender: null,
            send_time: '1',
            insert_time: '1',
            content: { kind: 'text', text: 'late preview' },
          },
        },
      ];
      if (hold) {
        hold = false;
        await new Promise<void>((resolve) => {
          release = resolve;
        });
      }
    }
    return reply;
  };
  await f.clock.advance(500);
  hold = true;
  f.service.invalidate('t0');
  await f.clock.advance(500);
  assert.ok(release);
  f.service.blockChannel('t0', id);
  release();
  await f.clock.advance(500);
  const entry = f.service.getSnapshot().get('t0');
  assert.equal(entry?.state, 'ready');
  assert.equal(entry?.data?.conversations[0].preview, null);
  assert.equal(entry?.data?.conversations[0].unread, '1');
  assert.equal(f.service.getSnapshot().get('t1')?.state, 'ready');
  assert.equal(f.waits.size, 1);
  f.bump('4');
  await f.clock.advance(1000);
  f.bump('2');
  await f.clock.advance(1000);
  assert.ok(f.service.isChannelBlocked('t0', id));
  assert.deepEqual(exclusions.at(-1), [id]);
  f.service.stop();
  f.service.updateStores(f.snapshot);
  f.service.start();
  await f.clock.advance(500);
  assert.equal(f.service.isChannelBlocked('t0', id), false);
  f.service.stop();
});

test('an unrefreshed profile asks for one catalog refresh per profile, not one per team or retry', async () => {
  const f = fixture();
  const requested: string[] = [];
  f.service.onCatalogRequired = (profile) => requested.push(profile);
  f.unrefreshed.add('t0');
  f.unrefreshed.add('t1');
  await f.clock.advance(500);
  assert.deepEqual(requested, ['p']);
  assert.equal(
    f.service.getSnapshot().get('t0')?.error,
    'The vault for p has not been refreshed. Refresh and try again.',
  );
  assert.equal(f.service.getSnapshot().get('t0')?.stale, true);
  // Retries inside the window are not repeated requests for the same profile.
  await f.clock.advance(5_000);
  assert.deepEqual(requested, ['p']);
  // A profile still unrefreshed after the window is asked for again.
  await f.clock.advance(20_000);
  assert.equal(requested.length, 2);
  // Once the vault is read again the teams synchronize without another ask.
  f.unrefreshed.clear();
  await f.clock.advance(30_000);
  assert.equal(f.service.getSnapshot().get('t0')?.error, '');
  assert.equal(requested.length, 2);
  f.service.stop();
});

test('a catalog read makes a team and its poll due at once instead of at their backoff', async () => {
  const f = fixture();
  f.service.onCatalogRequired = () => {};
  f.unrefreshed.add('t0');
  await f.clock.advance(4_000);
  // Five failed attempts have backed the team off past the next few seconds.
  const attempts = f.syncs.filter((id) => id === 't0').length;
  assert.equal(attempts, 5);
  assert.equal(f.service.getSnapshot().get('t0')?.stale, true);
  f.unrefreshed.clear();
  await f.clock.advance(300);
  assert.equal(f.syncs.filter((id) => id === 't0').length, attempts);
  // The vault is read again: the team synchronizes on the next drain rather
  // than when its backoff would have expired.
  f.service.updateStores(f.snapshot);
  await f.clock.advance(300);
  assert.equal(f.syncs.filter((id) => id === 't0').length, attempts + 1);
  assert.equal(f.service.getSnapshot().get('t0')?.error, '');
  assert.equal(f.service.getSnapshot().get('t0')?.stale, false);

  // The account's poll backs off the same way when the backend answers it
  // with the unrefreshed vault, and a catalog read brings it forward too.
  const refuse = () => {
    for (const [view, wait] of f.waits) {
      f.waits.delete(view);
      wait.reject({
        code: 'catalog-required',
        message:
          'The vault for p has not been refreshed. Refresh and try again.',
        retryable: true,
        fatal: false,
        ambiguous: false,
      });
    }
  };
  const polls = f.polls.length;
  assert.equal(f.waits.size, 1);
  // 250, 500, 1000, 2000 and 4000 ms between attempts: five refusals, five
  // polls issued again.
  for (let round = 0; round < 5; round++) {
    refuse();
    await f.clock.advance(4_300);
  }
  assert.equal(f.polls.length, polls + 5);
  assert.equal(f.waits.size, 1);
  // The sixth refusal backs the account off by eight seconds.
  refuse();
  await f.clock.advance(300);
  assert.equal(f.waits.size, 0);
  assert.equal(f.polls.length, polls + 5);
  f.service.updateStores(f.snapshot);
  await f.clock.advance(300);
  assert.equal(f.waits.size, 1);
  assert.equal(f.polls.length, polls + 6);
  f.service.stop();
});
test('a hidden window suspends the periodic resynchronization', async () => {
  const f = fixture();
  await f.clock.advance(500);
  assert.deepEqual(new Set(f.syncs), new Set(['t0', 't1']));
  f.service.setVisible(false);
  f.syncs.length = 0;
  await f.clock.advance(120_000);
  assert.deepEqual(f.syncs, []);
  // The account poll is the one round trip a hidden window keeps.
  assert.equal(f.waits.size, 1);
  f.service.stop();
});

test('a poll bump synchronizes its teams while the window is hidden', async () => {
  const f = fixture();
  await f.clock.advance(500);
  f.service.setVisible(false);
  f.syncs.length = 0;
  f.bump('2');
  await f.clock.advance(5_000);
  assert.deepEqual(new Set(f.syncs), new Set(['t0', 't1']));
  assert.equal(f.service.getSnapshot().get('t1')?.data?.head, '2');
  f.syncs.length = 0;
  await f.clock.advance(120_000);
  assert.deepEqual(f.syncs, []);
  f.service.stop();
});

test('showing the window again resynchronizes every team once', async () => {
  const f = fixture();
  await f.clock.advance(500);
  f.service.setVisible(false);
  await f.clock.advance(120_000);
  f.syncs.length = 0;
  f.service.setVisible(true);
  await f.clock.advance(500);
  assert.deepEqual(f.syncs.sort(), ['t0', 't1']);
  f.service.stop();
});

test('a hidden window drains every two seconds rather than every quarter second', async () => {
  const f = fixture();
  await f.clock.advance(500);
  const delays = () => {
    const seen: number[] = [];
    const before = f.clock.time;
    for (const task of f.clock.tasks.values()) seen.push(task.due - before);
    return seen;
  };
  assert.deepEqual(delays(), [250]);
  f.service.setVisible(false);
  await f.clock.advance(0);
  assert.deepEqual(delays(), [2_000]);
  f.service.setVisible(true);
  await f.clock.advance(0);
  assert.deepEqual(delays(), [250]);
  f.service.stop();
});

test('polls, synchronizations and arrivals are timed for diagnostics without channels or text', async () => {
  const f = fixture(1);
  const events: ChatInboxTiming[] = [];
  const stop = f.service.observe((event) => {
    events.push(event);
  });
  f.service.observe(() => {
    throw new Error('observer failure');
  });
  await f.clock.advance(500);
  const first = events.find((event) => event.kind === 'sync');
  assert.ok(first && first.kind === 'sync');
  assert.equal(first.outcome, 'ok');
  assert.equal(first.store, 't0');
  assert.equal(first.changed, false);
  assert.ok(!events.some((event) => event.kind === 'arrival'));
  events.length = 0;
  await f.clock.advance(1_000);
  f.bump('2');
  await f.clock.advance(1_000);
  const poll = events.find((event) => event.kind === 'poll');
  assert.ok(poll && poll.kind === 'poll');
  assert.equal(poll.bumped, true);
  assert.equal(poll.account, '1:p2:me');
  assert.ok(poll.milliseconds >= 1_000);
  const arrival = events.find((event) => event.kind === 'arrival');
  assert.ok(arrival && arrival.kind === 'arrival');
  assert.equal(arrival.store, 't0');
  assert.ok(arrival.milliseconds >= 0 && arrival.milliseconds <= 1_000);
  // One arrival per bump, not one per periodic resynchronization — which an
  // idle team now waits minutes for.
  events.length = 0;
  await f.clock.advance(310_000);
  assert.ok(events.some((event) => event.kind === 'sync'));
  assert.ok(!events.some((event) => event.kind === 'arrival'));
  stop();
  events.length = 0;
  await f.clock.advance(30_000);
  assert.equal(events.length, 0);
  f.service.stop();
});

test('a confirmed read publishes locally, is a floor, and costs no synchronization', async () => {
  const f = fixture(1);
  f.inbox.channels = ['c0'];
  f.inbox.position = 5;
  try {
    await f.clock.advance(500);
    const before = f.service.getSnapshot().get('t0')!;
    assert.equal(before.data?.conversations[0]?.unread, '5');
    const syncs = f.syncs.length;
    const revisions = [...(before.channelRefreshRevisions ?? [])];
    f.service.applyRead('t0', 'c0', '3');
    const after = f.service.getSnapshot().get('t0')!;
    assert.equal(after.data?.conversations[0]?.read_through, '3');
    assert.equal(after.data?.conversations[0]?.unread, '2');
    // The newest position the conversation states is unchanged, so the
    // channel is not invalidated and the thread reloads nothing.
    assert.deepEqual([...(after.channelRefreshRevisions ?? [])], revisions);
    assert.equal(f.service.isInvalidated('t0'), false);
    await f.clock.advance(60_000);
    assert.equal(
      f.syncs.length,
      syncs,
      'a confirmed read resynchronizes nothing',
    );
    // A reply that lands after another device read further does not roll the
    // pointer back, and neither does a repeat of the same sequence.
    f.service.applyRead('t0', 'c0', '2');
    f.service.applyRead('t0', 'c0', '3');
    const settled = f.service.getSnapshot().get('t0')!;
    assert.equal(settled.data?.conversations[0]?.read_through, '3');
    assert.equal(settled.data?.conversations[0]?.unread, '2');
  } finally {
    f.service.stop();
  }
});

test('a failed read invalidates the team, which is what a confirmed one no longer does', async () => {
  const f = fixture(1);
  f.inbox.channels = ['c0'];
  f.inbox.position = 5;
  try {
    await f.clock.advance(500);
    const syncs = f.syncs.length;
    // What `markRead` does on the failure branch, and only there.
    f.service.invalidate('t0');
    assert.equal(f.service.isInvalidated('t0'), true);
    await f.clock.advance(500);
    assert.equal(f.syncs.length, syncs + 1);
  } finally {
    f.service.stop();
  }
});

test('an idle team resynchronizes in minutes, and keeps the short cadence while its poll fails', async () => {
  const f = fixture(1);
  try {
    await f.clock.advance(500);
    assert.equal(f.syncs.length, 1);
    // What the twenty-five second cadence used to cost, and no longer does.
    await f.clock.advance(30_000);
    assert.equal(f.syncs.length, 1);
    await f.clock.advance(300_000);
    assert.equal(f.syncs.length, 2);
    // A poll that fails is the one case that cannot wait: nothing else
    // reports an arrival while it is down.
    const wait = f.waits.entries().next().value!;
    f.waits.delete(wait[0]);
    wait[1].reject({
      code: 'io',
      message: 'Connection lost.',
      retryable: true,
      fatal: false,
      ambiguous: false,
    });
    for (let i = 0; i < 30; i++) await Promise.resolve();
    await f.clock.advance(26_000);
    assert.ok(f.syncs.length >= 3, 'a failing poll keeps the short cadence');
  } finally {
    f.service.stop();
  }
});

test('a poll that recovers, a shown window and a local mutation each make the team due now', async () => {
  for (const trigger of ['recovery', 'focus', 'mutation', 'bump'] as const) {
    const f = fixture(1);
    try {
      await f.clock.advance(500);
      const syncs = f.syncs.length;
      if (trigger === 'recovery') {
        const wait = f.waits.entries().next().value!;
        f.waits.delete(wait[0]);
        wait[1].reject({
          code: 'io',
          message: 'Connection lost.',
          retryable: true,
          fatal: false,
          ambiguous: false,
        });
        for (let i = 0; i < 30; i++) await Promise.resolve();
        // The poll is retried and succeeds; the teams it could not report on
        // resynchronize at once rather than waiting out any interval.
        await f.clock.advance(1_000);
        f.bump('1');
      }
      if (trigger === 'focus') {
        f.service.setVisible(false);
        f.service.setVisible(true);
      }
      if (trigger === 'mutation') f.service.invalidate('t0');
      if (trigger === 'bump') f.bump('9');
      await f.clock.advance(600);
      assert.ok(
        f.syncs.length > syncs,
        `${trigger} must make the team due immediately`,
      );
    } finally {
      f.service.stop();
    }
  }
});

test('a new binding synchronizes immediately rather than waiting out the idle cadence', async () => {
  const f = fixture(1);
  try {
    await f.clock.advance(500);
    const syncs = f.syncs.length;
    const snapshot = structuredClone(f.snapshot);
    snapshot.servers[0].host_id = 'other-host';
    f.service.updateStores(snapshot, {}, new Map([['p', 0]]));
    await f.clock.advance(600);
    assert.ok(f.syncs.length > syncs);
  } finally {
    f.service.stop();
  }
});

test('a degraded projection holds the open channel at the base interval and backs the rest off', async () => {
  const f = fixture(1);
  f.inbox.channels = ['open', 'quiet-a', 'quiet-b'];
  f.inbox.degraded = true;
  f.inbox.position = 4;
  f.service.setOpenChannel('t0', 'open');
  const bumps = () =>
    new Map([
      ...(f.service.getSnapshot().get('t0')?.channelRefreshRevisions ?? []),
    ]);
  try {
    await f.clock.advance(500);
    const start = bumps();
    // Sixty seconds of synchronizations one second apart, which is what a
    // busy account's poll bumps produce.
    for (let step = 0; step < 60; step++) {
      f.service.invalidate('t0');
      await f.clock.advance(1_000);
    }
    const end = bumps();
    const raised = (id: string) => end.get(id)! - start.get(id)!;
    assert.ok(
      raised('open') >= 11,
      `the open channel must keep refreshing: ${raised('open')}`,
    );
    assert.ok(
      raised('quiet-a') <= 4 && raised('quiet-b') <= 4,
      `quiet channels must back off: ${raised('quiet-a')}`,
    );
    // An arrival the projection can see resets that channel to the base
    // interval, so a channel that starts receiving is not left backed off.
    const backedOff = bumps();
    f.inbox.position = 9;
    f.service.invalidate('t0');
    await f.clock.advance(1_000);
    assert.equal(bumps().get('quiet-a')! - backedOff.get('quiet-a')!, 1);
    const reset = bumps();
    for (let step = 0; step < 6; step++) {
      f.service.invalidate('t0');
      await f.clock.advance(1_000);
    }
    assert.equal(bumps().get('quiet-a')! - reset.get('quiet-a')!, 1);
  } finally {
    f.service.stop();
  }
});

test('a throttled degraded bump refreshes the open thread when its delay expires without another sync', async () => {
  const f = fixture(1);
  f.inbox.channels = ['open'];
  f.inbox.degraded = true;
  f.service.setOpenChannel('t0', 'open');
  const entry = () => f.service.getSnapshot().get('t0')!;
  const revision = () => entry().channelRefreshRevisions!.get('open')!;
  try {
    await f.clock.advance(500);
    f.bump('2');
    await f.clock.advance(1_000);
    const first = revision();
    const content = entry().channelRevisions.get('open');
    f.bump('3');
    await f.clock.advance(1_000);
    f.bump('4');
    await f.clock.advance(1_000);
    assert.equal(revision(), first, 'nearby bumps are coalesced');
    const syncs = f.syncs.length;
    await f.clock.advance(3_000);
    assert.equal(revision(), first + 1, 'the deferred refresh is not lost');
    assert.equal(f.syncs.length, syncs, 'releasing it needs no inbox RPC');
    assert.equal(entry().channelRevisions.get('open'), content);
    await f.clock.advance(6_000);
    assert.equal(
      revision(),
      first + 1,
      'one trailing refresh drains the bumps',
    );
  } finally {
    f.service.stop();
  }
});

test('recovery to a healthy projection clears deferred degraded refreshes', async () => {
  const f = fixture(1);
  f.inbox.channels = ['open'];
  f.inbox.degraded = true;
  f.service.setOpenChannel('t0', 'open');
  const revision = () =>
    f.service.getSnapshot().get('t0')!.channelRefreshRevisions!.get('open');
  try {
    await f.clock.advance(500);
    f.bump('2');
    await f.clock.advance(1_000);
    f.bump('3');
    await f.clock.advance(1_000);
    f.inbox.degraded = false;
    f.service.invalidate('t0');
    await f.clock.advance(500);
    const healthy = revision();
    await f.clock.advance(6_000);
    assert.equal(revision(), healthy);
    // A later degraded lifetime starts fresh, with no old delay to inherit.
    f.inbox.degraded = true;
    f.service.invalidate('t0');
    await f.clock.advance(500);
    assert.equal(revision(), healthy! + 1);
  } finally {
    f.service.stop();
  }
});
