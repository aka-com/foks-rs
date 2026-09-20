/** Inbox CPU microbenchmarks; run against current or --source <archive>. */
import { resolve, join } from 'node:path';
import { pathToFileURL } from 'node:url';
import assert from 'node:assert/strict';
import { notificationBenchmarkSnapshot } from './chat-notification-fixture';
import type { ChatAction } from '../../apps/desktop/src/chat-contract';
import type { Bridge } from '../../apps/desktop/src/bridge';
import type { TeamInbox } from '../../apps/desktop/src/chat/inbox-service';

const option = process.argv.indexOf('--source');
const root = resolve(option < 0 ? '.' : process.argv[option + 1]);
const { ChatInboxService } = await import(
  pathToFileURL(join(root, 'apps/desktop/src/chat/inbox-service.ts')).href
);

function incoming(count: number): TeamInbox {
  const channels = Array.from({ length: count }, (_, i) => ({
    id: `c${i}`,
    name: `channel-${i}`,
    description: null,
    admin: false,
    readable: true,
    writable: true,
    read_role: 'Member (0)',
    write_role: 'Member (0)',
  }));
  return {
    state: 'ready',
    error: '',
    note: '',
    stale: false,
    revision: 1,
    scope: {
      host: 'host',
      actor: 'actor',
      store: {
        profile: 'p',
        account_alias: 'a',
        team_alias: 't',
        team_id: 'team',
      },
    },
    blockedChannels: new Set(),
    channelRevisions: new Map(channels.map((c) => [c.id, 1])),
    channelRefreshRevisions: new Map(channels.map((c) => [c.id, 1])),
    data: {
      kind: 'inbox',
      channels,
      conversations: channels.map((channel) => ({
        // JSON bridge replies duplicate channel metadata across collections.
        channel: { ...channel },
        inbox_version: '1',
        read_through: '0',
        pending_read: null,
        unread: '100000',
        hidden: false,
        muted: false,
        preview: {
          sender: 'actor',
          send_time: '1',
          insert_time: '1',
          content: { kind: 'text', text: 'synthetic preview '.repeat(16) },
        },
      })),
      cursor: '1',
      head: '1',
      degraded: true,
      read_retry_pending: false,
      previews_incomplete: false,
      blocked_channels: [],
    },
  };
}
for (const count of [10, 100, 1_000]) {
  for (const workload of ['sync', 'read', 'revision'] as const) {
    const service = new ChatInboxService({} as Bridge);
    // Exercise the real publication boundary on both revisions without RPC,
    // revision comparison, timers, or a new production benchmark API.
    const boundary = service as unknown as {
      publishReply?: (id: string, entry: TeamInbox) => void;
      publish: (id: string, entry: TeamInbox, reason: string) => void;
    };
    const dto = incoming(count);
    const sync = () =>
      boundary.publishReply
        ? boundary.publishReply('t', dto)
        : boundary.publish('t', dto, 'sync');
    sync();
    let observed = 0;
    service.subscribe(() => {
      // Include one realistic O(channels) subscriber scan in both revisions.
      const entry: TeamInbox = service.getSnapshot().get('t')!;
      observed += entry.data!.conversations.reduce(
        (n, c) => n + Number(c.unread !== '0'),
        0,
      );
    });
    let sequence = 0;
    const run = () => {
      if (workload === 'sync') sync();
      else if (workload === 'read')
        service.applyRead('t', 'c0', String(++sequence));
      else {
        const old: TeamInbox = service.getSnapshot().get('t')!;
        const revisions = new Map(old.channelRefreshRevisions!);
        revisions.set('c0', (revisions.get('c0') ?? 0) + 1);
        boundary.publish(
          't',
          { ...old, channelRefreshRevisions: revisions },
          'degraded',
        );
      }
    };
    for (let i = 0; i < 20; i++) run();
    const batches = [];
    for (let batch = 0; batch < 5; batch++) {
      const start = performance.now();
      for (let i = 0; i < 100; i++) run();
      batches.push(((performance.now() - start) * 1000) / 100);
    }
    batches.sort((a, b) => a - b);
    console.log(
      JSON.stringify({
        channels: count,
        workload,
        medianMicroseconds: batches[2],
        maxBatchMeanMicroseconds: batches[4],
        batches: 5,
        iterations: 100,
        subscriberScans: observed / count,
      }),
    );
  }
}

// Include the full UI synchronization path separately: scheduling, acceptance,
// preview sanitization, revision calculation, publication and subscriber dispatch.
// The bridge is immediate; network, native execution and rendering are excluded.
for (const count of [10, 100, 1_000]) {
  const dto = incoming(count);
  dto.data!.degraded = false;
  let replies = 0;
  const service = new ChatInboxService(
    {
      chat: async (_id: string, action: ChatAction) => {
        assert.equal(action.action, 'sync-inbox');
        replies++;
        return { scope: dto.scope!, result: dto.data! };
      },
      cancelChat: async () => {},
    } as unknown as Bridge,
    {
      now: () => performance.now(),
      later: () => undefined,
      cancel: () => {},
      random: () => 0,
    },
  );
  service.updateStores(notificationBenchmarkSnapshot('t', dto.scope!));
  service.start();
  // Suppress polling/timers and invoke the real sync with its registered owner.
  const internal = service as unknown as {
    accounts: Map<string, { teams: Map<string, unknown> }>;
    sync: (account: unknown, team: unknown) => Promise<void>;
  };
  const account = [...internal.accounts.values()][0];
  assert.ok(account, 'fixture must be eligible for chat');
  const team = account.teams.get('t');
  let publications = 0;
  let observed = 0;
  const unsubscribe = service.subscribe(() => {
    const entry: TeamInbox = service.getSnapshot().get('t')!;
    assert.equal(entry.state, 'ready');
    assert.equal(entry.stale, false);
    assert.equal(entry.error, '');
    publications++;
    observed += entry.data!.conversations.reduce(
      (n, c) => n + Number(c.unread !== '0'),
      0,
    );
  });
  for (let i = 0; i < 20; i++) await internal.sync(account, team);
  const batches = [];
  for (let batch = 0; batch < 5; batch++) {
    const start = performance.now();
    for (let i = 0; i < 100; i++) await internal.sync(account, team);
    batches.push(((performance.now() - start) * 1000) / 100);
  }
  assert.equal(replies, 520);
  assert.equal(publications, replies, 'failed syncs must not look fast');
  batches.sort((a, b) => a - b);
  console.log(
    JSON.stringify({
      channels: count,
      workload: 'full-sync',
      medianMicroseconds: batches[2],
      maxBatchMeanMicroseconds: batches[4],
      batches: 5,
      iterations: 100,
      subscriberScans: observed / count,
    }),
  );
  unsubscribe();
  service.stop();
}
