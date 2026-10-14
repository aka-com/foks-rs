import assert from 'node:assert/strict';
import test from 'node:test';
import { incoming, notificationText } from '../src/chat/notification-policy';
import { CHAT_NOTIFICATION_TEXT_CHARS } from '../src/chat-limits';
import {
  decodeLocalSession,
  notificationKey,
} from '../src/chat/local-contract';
import type { ChatMessage, ChatScope, ChatTeamRef } from '../src/chat-contract';
import { decodedChatBridge } from './lib/decoded-chat-bridge';
// Consumer fakes reply through the native decoder, so every ID has the form
// the agent reports.
const ACTOR = '01' + 'aa'.repeat(32);
const OTHER = '01' + 'bb'.repeat(32);
const CHANNEL = 'cd'.repeat(16);
const team: ChatTeamRef = {
  profile: 'p',
  accountAlias: 'a',
  teamAlias: 't',
  teamId: '03' + 'ab'.repeat(32),
};
const scope: ChatScope = {
  host: '02' + 'ab'.repeat(32),
  actor: ACTOR,
  store: {
    profile: team.profile,
    account_alias: team.accountAlias,
    team_alias: team.teamAlias,
    team_id: team.teamId,
  },
};
const message = (
  sequence: number,
  sender: string | null = OTHER,
): ChatMessage => ({
  id: sequence.toString(16).padStart(32, '0'),
  sequence: String(sequence),
  sender,
  send_time: '1',
  insert_time: '1',
  content: { kind: 'text', text: 'hello' },
});
/**
 * A history result for at most `CHAT_PAGE_ROWS` rows: those below `before`,
 * with `before` naming the lowest returned sequence unless that is 1.
 */
const page = (channel: string, rows: ChatMessage[], before: string | null) => {
  const messages = rows.filter(
    (m) => before === null || BigInt(m.sequence) < BigInt(before),
  );
  const lowest = Math.min(...messages.map((m) => Number(m.sequence)));
  return {
    kind: 'history',
    channel,
    messages,
    before: messages.length && lowest > 1 ? String(lowest) : null,
    missing_predecessors: [],
  };
};
test('notification candidates exclude baseline, own, unknown, duplicate and visible messages', () => {
  const rows = [
    message(1),
    message(2, ACTOR),
    message(3, null),
    message(4),
    message(5),
    message(5),
    message(6),
  ];
  assert.deepEqual(
    incoming(rows, 1n, 5n, ACTOR, (id) => id === message(4).id).map(
      (m) => m.sequence,
    ),
    ['5'],
  );
  assert.equal(notificationText([], true).length, 0);
  const alerts = notificationText(
    Array.from({ length: 20 }, (_, i) => message(i + 1)),
    true,
  );
  assert.equal(alerts.length, 4);
  assert.equal(alerts[3].count, 17);
  assert.equal(alerts[3].incomplete, true);
});
test('notification bodies are cut to the shared bound in characters, not UTF-16 units', () => {
  const long = '\u{1F600}'.repeat(CHAT_NOTIFICATION_TEXT_CHARS + 1);
  const [alert] = notificationText(
    [{ ...message(1), content: { kind: 'text', text: long } }],
    false,
  );
  assert.equal(alert.body, '\u{1F600}'.repeat(CHAT_NOTIFICATION_TEXT_CHARS));
});
test('local preference keys bind actor, host, team and channel without persisting aliases', async () => {
  const key = await notificationKey(scope, 'c');
  assert.match(key, /^[a-f0-9]{64}\/[a-f0-9]{64}$/);
  for (const changed of [
    { ...scope, actor: 'other' },
    { ...scope, host: 'other' },
    { ...scope, store: { ...scope.store, team_id: 'other' } },
  ])
    assert.notEqual(await notificationKey(changed, 'c'), key);
  assert.notEqual(await notificationKey(scope, 'd'), key);
  assert.equal(
    await notificationKey(
      { ...scope, store: { ...scope.store, account_alias: 'renamed' } },
      'c',
    ),
    key,
  );
});
test('local settings decoder rejects invalid persisted identities and invalid flags', () => {
  const base = {
    epoch: 'a'.repeat(32),
    available: false,
    settings: { enabled: false, previews: false, overrides: {} },
  };
  assert.deepEqual(decodeLocalSession(base), base);
  assert.throws(() =>
    decodeLocalSession({
      ...base,
      settings: { ...base.settings, enabled: 'yes' },
    }),
  );
  assert.throws(() =>
    decodeLocalSession({
      ...base,
      settings: { ...base.settings, overrides: { alias: true } },
    }),
  );
});

test('consumer fakes reject a notification page the agent could not send', async () => {
  const reply = (text: string) => ({
    scope,
    result: page(
      CHANNEL,
      [{ ...message(2), content: { kind: 'text', text } }],
      null,
    ),
  });
  let next = reply('x'.repeat(CHAT_NOTIFICATION_TEXT_CHARS));
  const bridge = decodedChatBridge({ chat: async () => next }, team);
  const read = () =>
    bridge.chat('team:eng', {
      action: 'notification-history',
      channel: CHANNEL,
      before: null,
    });
  await read();
  // The agent cuts each notification text to CHAT_NOTIFICATION_TEXT_CHARS
  // characters.
  next = reply('x'.repeat(CHAT_NOTIFICATION_TEXT_CHARS + 1));
  await assert.rejects(read(), { code: 'chat-integrity' });
});

test('consumer baselines verified history, filters own messages and never writes read state', async () => {
  const { installDom } = await import('./lib/dom-harness');
  installDom({ url: 'http://localhost/', body: '<div></div>' });
  const { NotificationConsumer } =
    await import('../src/chat/notification-consumer');
  const channel = { id: CHANNEL, readable: true };
  let revision = 1;
  let rows = [message(10)];
  let historyCalls = 0;
  const alerts: unknown[] = [];
  const listeners = new Set<() => void>();
  const service = {
    handleError: () => false,
    openChannel: () => undefined,
    subscribe: (fn: () => void) => {
      listeners.add(fn);
      return () => listeners.delete(fn);
    },
    getSnapshot: () =>
      new Map([
        [
          'store',
          {
            state: 'ready',
            stale: false,
            note: '',
            scope,
            revision,
            channelRevisions: new Map([[CHANNEL, revision]]),
            blockedChannels: new Set(),
            data: {
              channels: [channel],
              // No conversation row states this channel's position, so the
              // baseline is established by a read rather than seeded.
              conversations: [],
            },
          },
        ],
      ]),
  };
  const bridge = decodedChatBridge(
    {
      chat: async (
        _store: string,
        action: { action: string; before: string | null },
      ) => {
        assert.equal(action.action, 'notification-history');
        historyCalls++;
        return { scope, result: page(CHANNEL, rows, action.before) };
      },
      cancelChat: async () => {},
      chatLocal: async (action: unknown) => {
        alerts.push(action);
      },
    },
    team,
  );
  const consumer = new NotificationConsumer(
    bridge as unknown as import('../src/bridge').Bridge,
    service as unknown as import('../src/chat/inbox-service').ChatInboxService,
    {
      epoch: 'a'.repeat(32),
      available: true,
      settings: { enabled: true, previews: false, overrides: {} },
    },
    () => {},
  );
  const until = async (check: () => boolean) => {
    for (let i = 0; i < 100 && !check(); i++)
      await new Promise((r) => setTimeout(r, 30));
    assert.ok(check());
  };
  try {
    await until(() => historyCalls === 1);
    assert.equal(alerts.length, 0);
    rows = [message(12), message(11, ACTOR), message(10)];
    revision++;
    for (const fn of listeners) fn();
    await until(() => alerts.length === 1);
    assert.equal((alerts[0] as { body?: string }).body, undefined);
    revision++;
    for (const fn of listeners) fn();
    await until(() => historyCalls === 3);
    assert.equal(alerts.length, 1);
  } finally {
    consumer.stop();
  }
});

test('consumer advances budgeted passes beyond 64 channels despite an always-failing first channel', async () => {
  const { NotificationConsumer } =
    await import('../src/chat/notification-consumer');
  const channels = Array.from({ length: 70 }, (_, i) => ({
    id: (i + 1).toString(16).padStart(32, 'c'),
    readable: true,
  }));
  const checked = new Set<string>();
  const budgets: { rows: number; bytes: number }[] = [];
  const service = {
    handleError: () => false,
    openChannel: () => undefined,
    subscribe: () => () => {},
    getSnapshot: () =>
      new Map([
        [
          'store',
          {
            state: 'ready',
            stale: false,
            note: '',
            scope,
            revision: 1,
            channelRevisions: new Map(channels.map((c) => [c.id, 1])),
            blockedChannels: new Set(),
            data: { channels, conversations: [] },
          },
        ],
      ]),
  };
  const bridge = decodedChatBridge(
    {
      chat: async (
        _store: string,
        action: { action: string; channel: string; before: string | null },
      ) => {
        assert.equal(action.action, 'notification-history');
        checked.add(action.channel);
        if (action.channel === channels[0].id)
          throw {
            code: 'offline',
            message: 'offline',
            fatal: false,
            retryable: true,
            ambiguous: false,
          };
        return {
          scope,
          result: page(
            action.channel,
            Array.from({ length: 50 }, (_, i) => message(50 - i)),
            action.before,
          ),
        };
      },
      cancelChat: async () => {},
      chatLocal: async () => {
        assert.fail('Baseline must not alert');
      },
    },
    team,
  );
  const consumer = new NotificationConsumer(
    bridge as unknown as import('../src/bridge').Bridge,
    service as unknown as import('../src/chat/inbox-service').ChatInboxService,
    {
      epoch: 'b'.repeat(32),
      available: true,
      settings: { enabled: true, previews: false, overrides: {} },
    },
    () => {},
    undefined,
    (event) => {
      if (event.kind === 'pass' && event.budgetHit) {
        budgets.push({ rows: event.rows, bytes: event.bytes });
      }
    },
  );
  try {
    for (let i = 0; i < 300 && checked.size < 70; i++)
      await new Promise((r) => setTimeout(r, 50));
    assert.equal(checked.size, 70);
    assert.ok(budgets.length > 0);
    assert.ok(
      budgets.every(
        (pass) => pass.rows <= 400 && pass.bytes <= 4 * 1024 * 1024,
      ),
    );
  } finally {
    consumer.stop();
  }
});

test('closing consumer during history discards late authorized plaintext', async () => {
  const { NotificationConsumer } =
    await import('../src/chat/notification-consumer');
  let finish: ((value: unknown) => void) | undefined;
  let alerts = 0;
  const service = {
    handleError: () => false,
    openChannel: () => undefined,
    subscribe: () => () => {},
    getSnapshot: () =>
      new Map([
        [
          'store',
          {
            state: 'ready',
            stale: false,
            note: '',
            scope,
            revision: 1,
            channelRevisions: new Map([[CHANNEL, 1]]),
            blockedChannels: new Set(),
            data: {
              channels: [{ id: CHANNEL, readable: true }],
              conversations: [],
            },
          },
        ],
      ]),
  };
  const bridge = decodedChatBridge(
    {
      chat: () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
      cancelChat: async () => {},
      chatLocal: async () => {
        alerts++;
      },
    },
    team,
  );
  const consumer = new NotificationConsumer(
    bridge as unknown as import('../src/bridge').Bridge,
    service as unknown as import('../src/chat/inbox-service').ChatInboxService,
    {
      epoch: 'b'.repeat(32),
      available: true,
      settings: { enabled: true, previews: false, overrides: {} },
    },
    () => {},
  );
  try {
    for (let i = 0; i < 50 && !finish; i++)
      await new Promise((r) => setTimeout(r, 20));
    assert.ok(finish);
    consumer.stop();
    finish({ scope, result: page(CHANNEL, [message(1)], null) });
    await new Promise((r) => setTimeout(r, 50));
    assert.equal(alerts, 0);
  } finally {
    consumer.stop();
  }
});

test('activating a notification opens the Chat tab on that team and channel', async () => {
  const { installDom } = await import('./lib/dom-harness');
  installDom({
    url: 'http://localhost/',
    body: '<div id="root"></div>',
    timers: true,
    act: true,
  });
  const ui = await import('@testing-library/react');
  const { createElement } = await import('react');
  const { createServer } = await import('vite');
  // The provider is JSX, so it is loaded the way the render tests load one.
  const vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
  const { NotificationProvider } = (await vite.ssrLoadModule(
    '/src/chat/notification-provider.tsx',
  )) as typeof import('../src/chat/notification-provider');
  const session = {
    epoch: 'a'.repeat(32),
    available: true,
    settings: { enabled: true, previews: false, overrides: {} },
  };
  const activation = { storeId: 'team:eng', channel: CHANNEL, scope };
  let taken = false;
  const navigations: unknown[] = [];
  const bridge = decodedChatBridge(
    {
      chatLocal: async (action: { action: string }) => {
        if (action.action !== 'take-activation') return session;
        const once = taken ? undefined : activation;
        taken = true;
        return { ...session, activation: once };
      },
      onChatNotification: async () => () => {},
      // The history read is the proof the activation still resolves; only
      // its scope decides whether the navigation happens.
      chat: async (_store: string, action: { action: string }) => {
        assert.equal(action.action, 'history');
        return { scope, result: page(activation.channel, [], null) };
      },
      cancelChat: async () => {},
    },
    team,
  );
  const service = {
    handleError: () => false,
    openChannel: () => undefined,
    subscribe: () => () => {},
    getSnapshot: () => new Map(),
  };
  try {
    ui.render(
      createElement(NotificationProvider, {
        bridge: bridge as unknown as import('../src/bridge').Bridge,
        service:
          service as unknown as import('../src/chat/inbox-service').ChatInboxService,
        onNavigate: (location: unknown) => navigations.push(location),
        children: null,
      }),
    );
    await ui.waitFor(() =>
      assert.deepEqual(navigations, [
        { kind: 'chat', ref: activation.storeId, channel: activation.channel },
      ]),
    );
  } finally {
    ui.cleanup();
    await vite.close();
  }
});

test('NotificationProvider builds its consumer on the clock it is given', async () => {
  const { installDom } = await import('./lib/dom-harness');
  installDom({
    url: 'http://localhost/',
    body: '<div id="root"></div>',
    timers: true,
    act: true,
  });
  const ui = await import('@testing-library/react');
  const { createElement } = await import('react');
  const { createServer } = await import('vite');
  const vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
  const { NotificationProvider } = (await vite.ssrLoadModule(
    '/src/chat/notification-provider.tsx',
  )) as typeof import('../src/chat/notification-provider');
  const session = {
    epoch: 'a'.repeat(32),
    available: true,
    settings: { enabled: true, previews: false, overrides: {} },
  };
  const bridge = {
    chatLocal: async () => session,
    onChatNotification: async () => () => {},
    chat: async () => {
      throw new Error('no channel is listed');
    },
    cancelChat: async () => {},
  };
  const service = {
    handleError: () => false,
    openChannel: () => undefined,
    subscribe: () => () => {},
    getSnapshot: () => new Map(),
  };
  // A consumer built with the default clock reads the wall clock and never
  // calls this one.
  const reads: number[] = [];
  const clock = {
    now: () => {
      reads.push(1);
      return 1_000;
    },
    later: () => 0,
    cancel: () => {},
    random: () => 0,
  };
  try {
    ui.render(
      createElement(NotificationProvider, {
        bridge: bridge as unknown as import('../src/bridge').Bridge,
        service:
          service as unknown as import('../src/chat/inbox-service').ChatInboxService,
        clock,
        children: null,
      }),
    );
    await ui.waitFor(() => assert.ok(reads.length > 0));
  } finally {
    ui.cleanup();
    await vite.close();
  }
});

test('notification activation validates the full store binding before navigation', () => {
  const store = {
    profile: 'p',
    account_alias: 'a',
    team_alias: 't',
    team_id: '03' + 'ab'.repeat(32),
  };
  const storeId = JSON.stringify({
    kind: 'team',
    profile: 'p',
    accountAlias: 'a',
    teamAlias: 't',
    teamId: store.team_id,
  });
  const activation = {
    storeId,
    channel: 'cd'.repeat(16),
    scope: {
      store,
      host: '02' + 'ab'.repeat(32),
      actor: '01' + 'ab'.repeat(32),
    },
  };
  const response = {
    epoch: 'aa'.repeat(16),
    available: true,
    settings: { enabled: true, previews: false, overrides: {} },
    activation,
  };
  assert.deepEqual(decodeLocalSession(response).activation, activation);
  assert.throws(() =>
    decodeLocalSession({
      ...response,
      activation: { ...activation, storeId: storeId.replace('"a"', '"other"') },
    }),
  );
  assert.throws(() =>
    decodeLocalSession({
      ...response,
      activation: { ...activation, channel: '../channel' },
    }),
  );
});
