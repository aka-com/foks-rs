import assert from 'node:assert/strict';
import test from 'node:test';
import { PROTOCOL_CAPABILITIES } from '../src/model/types';
import { cancelled } from '../src/chat/client';
import { ChatInboxService } from '../src/chat/inbox-service';
import type { ChatClock } from '../src/chat/inbox-service';
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
  const fail = new Set<string>();
  const denied = new Set<string>();
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
      return {
        scope: scope(store),
        result: {
          kind: 'inbox',
          channels: [],
          conversations: [],
          cursor: head,
          head,
          degraded: false,
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
    syncs,
    polls,
    fail,
    denied,
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
  assert.ok(registrations > 128);
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
