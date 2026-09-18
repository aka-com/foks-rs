import assert from 'node:assert/strict';
import test from 'node:test';
import { incoming, notificationText } from '../src/chat/notification-policy';
import {
  decodeLocalSession,
  notificationKey,
} from '../src/chat/local-contract';
import type { ChatMessage, ChatScope } from '../src/chat-contract';
const scope: ChatScope = {
  host: 'host',
  actor: 'actor',
  store: { profile: 'p', account_alias: 'a', team_alias: 't', team_id: 'team' },
};
const message = (id: number, sender: string | null = 'other'): ChatMessage => ({
  id: String(id),
  sequence: String(id),
  sender,
  send_time: '1',
  insert_time: '1',
  content: { kind: 'text', text: 'hello' },
});
test('notification candidates exclude baseline, own, unknown, duplicate and visible messages', () => {
  const rows = [
    message(1),
    message(2, 'actor'),
    message(3, null),
    message(4),
    message(5),
    message(5),
    message(6),
  ];
  assert.deepEqual(
    incoming(rows, 1n, 5n, 'actor', (id) => id === '4').map((m) => m.id),
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

test('consumer baselines verified history, filters own messages and never writes read state', async () => {
  const { installDom } = await import('./lib/dom-harness');
  installDom({ url: 'http://localhost/', body: '<div></div>' });
  const { NotificationConsumer } =
    await import('../src/chat/notification-consumer');
  const channel = { id: 'c', readable: true };
  let revision = 1;
  let rows = [message(10)];
  let historyCalls = 0;
  const alerts: unknown[] = [];
  const listeners = new Set<() => void>();
  const service = {
    handleError: () => false,
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
            scope,
            revision,
            channelRevisions: new Map([['c', revision]]),
            blockedChannels: new Set(),
            data: {
              channels: [channel],
              conversations: [
                { channel, muted: false, hidden: false, unread: '100000' },
              ],
            },
          },
        ],
      ]),
  };
  const bridge = {
    chat: async (_store: string, action: { action: string }) => {
      assert.equal(action.action, 'notification-history');
      historyCalls++;
      return {
        scope,
        result: {
          kind: 'history',
          channel: 'c',
          messages: rows,
          before: null,
          missing_predecessors: [],
        },
      };
    },
    cancelChat: async () => {},
    chatLocal: async (action: unknown) => {
      alerts.push(action);
    },
  };
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
    rows = [message(12), message(11, 'actor'), message(10)];
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

test('consumer rotates beyond 64 channels despite an always-failing first channel', async () => {
  const { NotificationConsumer } =
    await import('../src/chat/notification-consumer');
  const channels = Array.from({ length: 70 }, (_, i) => ({
    id: String(i),
    readable: true,
  }));
  const checked = new Set<string>();
  const service = {
    handleError: () => false,
    subscribe: () => () => {},
    getSnapshot: () =>
      new Map([
        [
          'store',
          {
            state: 'ready',
            stale: false,
            scope,
            revision: 1,
            channelRevisions: new Map(channels.map((c) => [c.id, 1])),
            blockedChannels: new Set(),
            data: { channels, conversations: [] },
          },
        ],
      ]),
  };
  const bridge = {
    chat: async (
      _store: string,
      action: { action: string; channel: string },
    ) => {
      assert.equal(action.action, 'notification-history');
      checked.add(action.channel);
      if (action.channel === '0')
        throw {
          code: 'offline',
          message: 'offline',
          fatal: false,
          retryable: true,
          ambiguous: false,
        };
      return {
        scope,
        result: {
          kind: 'history',
          channel: action.channel,
          messages: [],
          before: null,
          missing_predecessors: [],
        },
      };
    },
    cancelChat: async () => {},
    chatLocal: async () => {
      assert.fail('Baseline must not alert');
    },
  };
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
    for (let i = 0; i < 300 && checked.size < 70; i++)
      await new Promise((r) => setTimeout(r, 50));
    assert.equal(checked.size, 70);
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
    subscribe: () => () => {},
    getSnapshot: () =>
      new Map([
        [
          'store',
          {
            state: 'ready',
            stale: false,
            scope,
            revision: 1,
            channelRevisions: new Map([['c', 1]]),
            blockedChannels: new Set(),
            data: {
              channels: [{ id: 'c', readable: true }],
              conversations: [],
            },
          },
        ],
      ]),
  };
  const bridge = {
    chat: () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
    cancelChat: async () => {},
    chatLocal: async () => {
      alerts++;
    },
  };
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
    finish({
      scope,
      result: {
        kind: 'history',
        channel: 'c',
        messages: [message(1)],
        before: null,
        missing_predecessors: [],
      },
    });
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
  const activation = { storeId: 'team:eng', channel: 'cd'.repeat(16), scope };
  let taken = false;
  const navigations: unknown[] = [];
  const bridge = {
    chatLocal: async (action: { action: string }) => {
      if (action.action !== 'take-activation') return session;
      const once = taken ? undefined : activation;
      taken = true;
      return { ...session, activation: once };
    },
    onChatNotification: async () => () => {},
    // The history read is the proof the activation still resolves; only its
    // scope decides whether the navigation happens.
    chat: async (_store: string, action: { action: string }) => {
      assert.equal(action.action, 'history');
      return {
        scope,
        result: {
          kind: 'history',
          channel: activation.channel,
          messages: [],
          before: null,
          missing_predecessors: [],
        },
      };
    },
    cancelChat: async () => {},
  };
  const service = {
    handleError: () => false,
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
