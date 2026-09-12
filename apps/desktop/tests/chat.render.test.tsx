import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, StrictMode, useState } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import { installDom } from './lib/dom-harness';
import type { Location } from '../src/location';
import type { Bridge } from '../src/bridge';
installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
});
let ui: typeof import('@testing-library/react');
let vite: ViteDevServer;
test.before(async () => {
  ui = await import('@testing-library/react');
  vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
});
test.afterEach(() => ui.cleanup());
test.after(async () => {
  await vite.close();
});
async function setup(
  override?: (b: Bridge) => Bridge,
  waitForHistory = true,
  onNavigate: (location: unknown) => void = () => {},
  showChat = true,
  navigate = false,
) {
  const { Sidebar } = await vite.ssrLoadModule('/src/shell/sidebar.tsx');
  const { ChatInboxProvider } = await vite.ssrLoadModule(
    '/src/chat/inbox-provider.tsx',
  );
  const { ChatScreen } = await vite.ssrLoadModule(
    '/src/screens/chat-screen.tsx',
  );
  const { OverlayProvider } = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const enabledWorld = {
    ...FIXTURE,
    stores: FIXTURE.stores.filter(
      (s) => s.kind !== 'team' || s.id === 'team:eng',
    ),
    servers: FIXTURE.servers.map((s) =>
      s.id === 'acme' ? { ...s, chat_available: true } : s,
    ),
  };
  const baseBridge = mockBridge(enabledWorld);
  const bridge = override?.(baseBridge) ?? baseBridge;
  const portalRoot = document.getElementById('overlays');
  if (!portalRoot) throw new Error('missing overlay root');
  const overlayRoot = portalRoot;
  function Host() {
    const [location, setLocation] = useState<Location>({
      kind: 'team-chat',
      ref: 'team:eng',
    });
    return createElement(
      StrictMode,
      null,
      createElement(OverlayProvider, {
        backgroundRef: { current: null },
        portalRoot: overlayRoot,
        children: createElement(ChatInboxProvider, {
          bridge,
          world: enabledWorld,
          children: showChat
            ? createElement(ChatScreen, {
                world: enabledWorld,
                bridge,
                location,
                onNavigate: (next: Location) => {
                  onNavigate(next);
                  if (navigate) setLocation(next);
                },
              })
            : createElement(Sidebar, {
                world: enabledWorld,
                location: { kind: 'all' },
                alerts: 0,
                onNavigate,
              }),
        }),
      }),
    );
  }
  const rendered = ui.render(createElement(Host));
  if (waitForHistory) await ui.screen.findByText('Team chat is ready.');
  return rendered;
}
function openChannelSheet() {
  ui.fireEvent.click(
    ui.screen.getAllByRole('button', { name: 'New channel' })[0],
  );
  return ui.screen.getByRole('textbox', { name: 'Channel name' });
}
test('composer suppresses IME and repeated Enter, and renders hostile text safely', async () => {
  let preparations = 0;
  let attempts = 0;
  await setup((base) => ({
    ...base,
    chat: async (store, action) => {
      if (action.action === 'prepare-message') preparations++;
      if (action.action === 'attempt') attempts++;
      return base.chat(store, action);
    },
  }));
  const composer = ui.screen.getByRole('textbox', { name: 'Message' });
  ui.fireEvent.change(composer, {
    target: { value: '<img src=x onerror=alert(1)>' },
  });
  ui.fireEvent.keyDown(composer, {
    key: 'Enter',
    isComposing: true,
    keyCode: 229,
  });
  assert.equal(preparations, 0);
  ui.fireEvent.keyDown(composer, { key: 'Enter' });
  ui.fireEvent.keyDown(composer, { key: 'Enter' });
  await ui.screen.findByText('<img src=x onerror=alert(1)>');
  assert.equal(preparations, 1);
  assert.equal(attempts, 1);
  assert.equal(document.querySelector('.chat-messages img'), null);
  assert.equal((composer as HTMLTextAreaElement).value, '');
});
test('renders channel descriptions, message times, and bounded inbox previews', async () => {
  await setup();
  await ui.screen.findByText('A place for the whole team.');
  assert.ok(
    ui.screen.getByText('Team member: Team chat is ready.', { exact: true }),
  );
  const time = document.querySelector<HTMLTimeElement>('.chat-message time');
  assert.equal(time?.dateTime, '2023-11-14T22:13:20.001Z');
  assert.match(time?.title ?? '', /inserted as message 1/);
});

test('conversation inbox wakes, refreshes history, and shows unread state', async () => {
  const originalFocus = document.hasFocus.bind(document);
  Object.defineProperty(document, 'hasFocus', {
    configurable: true,
    value: () => false,
  });
  let direct!: Bridge['chat'];
  let incoming = false;
  let polls = 0;
  try {
    await setup((base) => {
      direct = (store, action, view) => base.chat(store, action, view);
      return {
        ...base,
        chat: async (store, action, view) => {
          if (action.action === 'poll-inbox') polls++;
          const reply = await base.chat(store, action, view);
          if (reply.result.kind === 'inbox')
            reply.result.conversations = reply.result.conversations.map(
              (conversation) => ({
                ...conversation,
                unread: incoming ? '1' : '0',
              }),
            );
          return reply;
        },
      };
    });
    assert.equal(ui.screen.queryByLabelText('1 unread'), null);
    incoming = true;
    const prepared = await direct('team:eng', {
      action: 'prepare-message',
      submission: '88'.repeat(16),
      channel: 'ab'.repeat(16),
      text: 'Arrived through live sync',
    });
    assert.equal(prepared.result.kind, 'operation');
    if (prepared.result.kind !== 'operation')
      throw new Error('missing operation');
    await direct('team:eng', {
      action: 'attempt',
      operation: prepared.result.operation.id,
    });
    await ui.screen.findByText('Arrived through live sync');
    assert.ok(polls > 0);
    assert.ok(ui.screen.getByLabelText('1 unread'));
  } finally {
    Object.defineProperty(document, 'hasFocus', {
      configurable: true,
      value: originalFocus,
    });
  }
});

test('live sync backs off across repeated post-poll sync failures', async () => {
  const syncs: number[] = [];
  await setup((base) => ({
    ...base,
    chat: async (store, action, view) => {
      if (action.action === 'sync-inbox') {
        syncs.push(performance.now());
        if (syncs.length > 1)
          throw {
            code: 'busy',
            message: 'Sync remains unavailable',
            fatal: false,
            retryable: true,
            ambiguous: false,
          };
      }
      return base.chat(store, action, view);
    },
  }));
  await ui.screen.findByText('Live updates paused');
  await ui.waitFor(() => assert.ok(syncs.length >= 4), { timeout: 4_000 });
  assert.ok(syncs[3] - syncs[2] >= 450);
});

test('read markers require focus and the newest displayed position', async () => {
  const originalFocus = document.hasFocus.bind(document);
  Object.defineProperty(document, 'hasFocus', {
    configurable: true,
    value: () => true,
  });
  let direct!: Bridge['chat'];
  const marks: string[] = [];
  try {
    await setup((base) => {
      direct = (store, action, view) => base.chat(store, action, view);
      return {
        ...base,
        chat: async (store, action, view) => {
          if (action.action === 'mark-read') marks.push(action.sequence);
          return base.chat(store, action, view);
        },
      };
    });
    await ui.waitFor(() => assert.equal(marks.length, 1));
    const scroller = ui.screen.getByLabelText('Message history');
    Object.defineProperty(scroller, 'scrollHeight', {
      configurable: true,
      value: 1_000,
    });
    Object.defineProperty(scroller, 'clientHeight', {
      configurable: true,
      value: 100,
    });
    scroller.scrollTop = 0;
    ui.fireEvent.scroll(scroller);
    const prepared = await direct('team:eng', {
      action: 'prepare-message',
      submission: '99'.repeat(16),
      channel: 'ab'.repeat(16),
      text: 'Do not mark while reading old history',
    });
    if (prepared.result.kind !== 'operation')
      throw new Error('missing operation');
    await direct('team:eng', {
      action: 'attempt',
      operation: prepared.result.operation.id,
    });
    await ui.screen.findByText('Do not mark while reading old history');
    await new Promise((resolve) => setTimeout(resolve, 400));
    assert.deepEqual(marks, ['1']);
  } finally {
    Object.defineProperty(document, 'hasFocus', {
      configurable: true,
      value: originalFocus,
    });
  }
});

test('lost preparation reply reuses its submission and sends exactly once', async () => {
  const submissions: string[] = [];
  let attempts = 0;
  let dropped = false;
  await setup((base) => ({
    ...base,
    chat: async (store, action) => {
      const reply = await base.chat(store, action);
      if (action.action === 'prepare-message') {
        submissions.push(action.submission);
        if (!dropped) {
          dropped = true;
          throw {
            code: 'ambiguous',
            message: 'Preparation reply lost',
            ambiguous: true,
            retryable: false,
            fatal: false,
          };
        }
      }
      if (action.action === 'attempt') attempts++;
      return reply;
    },
  }));
  ui.fireEvent.change(ui.screen.getByRole('textbox', { name: 'Message' }), {
    target: { value: 'recover this message' },
  });
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Send' }));
  await ui.screen.findByText('Preparation reply lost');
  assert.equal(attempts, 0);
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Recover preparation' }),
  );
  await ui.screen.findByText('recover this message');
  assert.equal(new Set(submissions).size, 1);
  assert.equal(submissions.length, 2);
  assert.equal(attempts, 1);
});
test('closing a conversation during preparation never starts delivery', async () => {
  let release!: () => void;
  let prepared!: () => void;
  const reached = new Promise<void>((resolve) => {
    prepared = resolve;
  });
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  let attempts = 0;
  const rendered = await setup((base) => ({
    ...base,
    chat: async (store, action) => {
      const result = await base.chat(store, action);
      if (action.action === 'prepare-message') {
        prepared();
        await gate;
      }
      if (action.action === 'attempt') attempts++;
      return result;
    },
  }));
  ui.fireEvent.change(ui.screen.getByRole('textbox', { name: 'Message' }), {
    target: { value: 'leave prepared' },
  });
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Send' }));
  await reached;
  rendered.unmount();
  await ui.act(async () => {
    release();
    await gate;
  });
  assert.equal(attempts, 0);
  assert.equal(document.body.textContent?.includes('leave prepared'), false);
});

test('definite preparation errors allow input correction', async () => {
  await setup((base) => ({
    ...base,
    chat: async (store, action) => {
      if (action.action === 'prepare-channel')
        throw {
          code: 'chat-invalid-input',
          message: 'Use an empty name for general.',
          fatal: false,
          retryable: false,
          ambiguous: false,
        };
      return base.chat(store, action);
    },
  }));
  const name = openChannelSheet();
  ui.fireEvent.change(name, { target: { value: 'general' } });
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Create channel' }));
  await ui.screen.findByText('Use an empty name for general.');
  assert.equal((name as HTMLInputElement).disabled, false);
  ui.fireEvent.change(name, { target: { value: 'valid' } });
  assert.equal((name as HTMLInputElement).value, 'valid');
  assert.ok(ui.screen.getByRole('dialog', { name: 'New channel' }));
});

test('ambiguous channel preparation keeps its sheet and submission until recovery', async () => {
  const submissions: string[] = [];
  const descriptions: string[] = [];
  let dropped = false;
  await setup((base) => ({
    ...base,
    chat: async (store, action, view) => {
      const reply = await base.chat(store, action, view);
      if (action.action === 'prepare-channel') {
        submissions.push(action.submission);
        descriptions.push(action.description);
        if (!dropped) {
          dropped = true;
          throw {
            code: 'ambiguous',
            message: 'Preparation reply lost',
            ambiguous: true,
            retryable: false,
            fatal: false,
          };
        }
      }
      return reply;
    },
  }));
  const name = openChannelSheet();
  ui.fireEvent.change(name, { target: { value: 'recoverable' } });
  const description = ui.screen.getByRole('textbox', {
    name: 'Channel description',
  });
  ui.fireEvent.change(description, { target: { value: 'Team Decisions' } });
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Create channel' }));
  await ui.screen.findByText('Preparation reply lost');
  assert.equal((description as HTMLTextAreaElement).disabled, true);
  assert.equal(
    ui.screen.getByRole('button', { name: 'Cancel' }).hasAttribute('disabled'),
    true,
  );
  ui.fireEvent.keyDown(document, { key: 'Escape' });
  assert.ok(ui.screen.getByRole('dialog', { name: 'New channel' }));
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Recover preparation' }),
  );
  await ui.screen.findByRole('button', { name: /# recoverable/ });
  await ui.waitFor(() => assert.equal(ui.screen.queryByRole('dialog'), null));
  assert.equal(new Set(submissions).size, 1);
  assert.equal(submissions.length, 2);
  assert.deepEqual(descriptions, ['Team Decisions', 'Team Decisions']);
});

test('team members opens the existing membership workflow in the same window', async () => {
  let destination: unknown;
  await setup(undefined, true, (location) => {
    destination = location;
  });
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Team members' }));
  assert.deepEqual(destination, {
    kind: 'group-settings',
    ref: 'team:eng',
    tab: 'people',
  });
});

test('reopening recovers retained pending text without attempting the send', async () => {
  let seeded = false;
  let attempts = 0;
  await setup((base) => ({
    ...base,
    chat: async (store, action, view) => {
      if (action.action === 'attempt') attempts++;
      if (action.action === 'pending' && !seeded) {
        seeded = true;
        const channels = await base.chat(store, { action: 'channels' }, view);
        assert.equal(channels.result.kind, 'channels');
        if (channels.result.kind !== 'channels') throw new Error('fixture');
        await base.chat(
          store,
          {
            action: 'prepare-message',
            submission: 'ac'.repeat(16),
            channel: channels.result.channels[0].id,
            text: 'Recovered original pending text',
          },
          view,
        );
      }
      return base.chat(store, action, view);
    },
  }));
  await ui.screen.findByText('Recovered original pending text');
  assert.equal(attempts, 0);
});

test('channel creation opens in a sheet, selects the new channel, and needs no manual cleanup', async () => {
  const navigations: unknown[] = [];
  await setup(undefined, true, (location) => navigations.push(location));
  assert.equal(ui.screen.queryByRole('dialog'), null);
  const name = openChannelSheet();
  assert.equal(document.activeElement, name);
  const audience = ui.screen.getByRole('radiogroup', {
    name: 'Channel audience',
  });
  assert.ok(audience);
  assert.equal(
    ui.screen
      .getByRole('radio', { name: /Everyone on the team/ })
      .getAttribute('aria-checked'),
    'true',
  );
  ui.fireEvent.change(name, { target: { value: 'design' } });
  ui.fireEvent.submit(name.closest('form')!);
  await ui.screen.findByRole('button', { name: /# design/ });
  await ui.waitFor(() => assert.equal(ui.screen.queryByRole('dialog'), null));
  assert.equal(
    ui.screen.queryByRole('button', { name: 'Finish cleanup' }),
    null,
  );
  assert.equal(ui.screen.queryByText('Needs attention'), null);
  assert.deepEqual(navigations.at(-1), {
    kind: 'team-chat',
    ref: 'team:eng',
    channel: '0000000000000000000000000000000b',
  });
});

test('own messages read as You and unread messages sit under a New divider', async () => {
  const originalFocus = document.hasFocus.bind(document);
  Object.defineProperty(document, 'hasFocus', {
    configurable: true,
    value: () => false,
  });
  const actor = '01' + 'ab'.repeat(32);
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const namedSender = FIXTURE.parties.find(
    (party) => party.store === 'team:eng' && party.username,
  );
  assert.ok(namedSender?.username);
  const other = namedSender.party_id_hex;
  try {
    await setup((base) => ({
      ...base,
      chat: async (store, action, view) => {
        const reply = await base.chat(store, action, view);
        if (action.action === 'history' && reply.result.kind === 'history')
          reply.result.messages = [
            {
              id: '01'.repeat(16),
              sequence: '1',
              sender: actor,
              send_time: '1700000000000',
              insert_time: '1700000000001',
              content: { kind: 'text', text: 'Team chat is ready.' },
            },
            {
              id: '02'.repeat(16),
              sequence: '2',
              sender: other,
              send_time: '1700000000000',
              insert_time: '1700000000001',
              content: { kind: 'text', text: 'Second message' },
            },
            {
              id: '03'.repeat(16),
              sequence: '3',
              sender: other,
              send_time: '1700000000000',
              insert_time: '1700000000001',
              content: { kind: 'text', text: 'Third message' },
            },
          ];
        if (reply.result.kind === 'inbox')
          reply.result.conversations = reply.result.conversations.map(
            (conversation) => ({
              ...conversation,
              read_through: '1',
              unread: '2',
            }),
          );
        return reply;
      },
    }));
    await ui.screen.findByText('Third message');
    assert.equal(ui.screen.getByText('You').getAttribute('title'), actor);
    assert.equal(ui.screen.getAllByText(namedSender.username).length, 2);
    await ui.screen.findByRole('separator', { name: 'New messages' });
    const rows = [...document.querySelectorAll('.chat-divider, .chat-message')];
    assert.deepEqual(
      rows.map((row) => row.className),
      ['chat-message', 'chat-divider', 'chat-message', 'chat-message'],
    );
  } finally {
    Object.defineProperty(document, 'hasFocus', {
      configurable: true,
      value: originalFocus,
    });
  }
});

test('scrolling away from the newest message displays a Jump to latest button', async () => {
  await setup();
  assert.equal(
    ui.screen.queryByRole('button', { name: 'Jump to latest' }),
    null,
  );
  const scroller = ui.screen.getByLabelText('Message history');
  Object.defineProperty(scroller, 'scrollHeight', {
    configurable: true,
    value: 1_000,
  });
  Object.defineProperty(scroller, 'clientHeight', {
    configurable: true,
    value: 100,
  });
  scroller.scrollTop = 0;
  ui.fireEvent.scroll(scroller);
  ui.fireEvent.click(
    await ui.screen.findByRole('button', { name: 'Jump to latest' }),
  );
  assert.equal(scroller.scrollTop, 1_000);
  await ui.waitFor(() =>
    assert.equal(
      ui.screen.queryByRole('button', { name: 'Jump to latest' }),
      null,
    ),
  );
});

test('channel listing failures offer a retry beside the message', async () => {
  let online = false;
  await setup(
    (base) => ({
      ...base,
      chat: async (store, action, view) => {
        if (action.action === 'sync-inbox' && !online) {
          throw {
            code: 'offline',
            message: 'Offline',
            fatal: false,
            retryable: true,
            ambiguous: false,
          };
        }
        return base.chat(store, action, view);
      },
    }),
    false,
  );
  await ui.screen.findByText('Offline');
  online = true;
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Retry' }));
  await ui.screen.findByText('Team chat is ready.');
  assert.equal(ui.screen.queryByText('Offline'), null);
});

test('prepending older messages preserves scroll position', async () => {
  await setup((base) => ({
    ...base,
    chat: async (store, action) => {
      const reply = await base.chat(store, action);
      if (action.action === 'history' && reply.result.kind === 'history') {
        reply.result.messages =
          action.before === null
            ? [
                {
                  id: 'cd'.repeat(16),
                  sequence: '3',
                  sender: null,
                  send_time: '1700000000000',
                  insert_time: '1700000000001',
                  content: { kind: 'text', text: 'Team chat is ready.' },
                },
              ]
            : [
                {
                  id: '01'.repeat(16),
                  sequence: '1',
                  sender: null,
                  send_time: '1700000000000',
                  insert_time: '1700000000001',
                  content: { kind: 'text', text: 'First earlier message' },
                },
                {
                  id: '02'.repeat(16),
                  sequence: '2',
                  sender: null,
                  send_time: '1700000000000',
                  insert_time: '1700000000001',
                  content: { kind: 'text', text: 'Second earlier message' },
                },
              ];
        reply.result.before = action.before === null ? '3' : null;
      }
      return reply;
    },
  }));
  const scroll = ui.screen.getByLabelText('Message history');
  Object.defineProperty(scroll, 'scrollHeight', {
    configurable: true,
    get: () => document.querySelectorAll('.chat-message').length * 100,
  });
  scroll.scrollTop = 20;
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Load older messages' }),
  );
  await ui.screen.findByText('First earlier message');
  assert.equal(scroll.scrollTop, 220);
  assert.deepEqual(
    [...document.querySelectorAll('.chat-message p')].map((p) => p.textContent),
    ['First earlier message', 'Second earlier message', 'Team chat is ready.'],
  );
});

test('refresh resets a disjoint window so older pagination reaches the gap', async () => {
  let latest = 100;
  const cursors: (string | null)[] = [];
  await setup((base) => ({
    ...base,
    chat: async (store, action) => {
      const reply = await base.chat(store, action);
      if (action.action === 'history' && reply.result.kind === 'history') {
        cursors.push(action.before);
        const end = action.before ? Number(action.before) - 1 : latest;
        reply.result.messages = Array.from({ length: 50 }, (_, i) => {
          const n = end - 49 + i;
          return {
            id: n.toString(16).padStart(32, '0'),
            sequence: String(n),
            sender: null,
            send_time: '1700000000000',
            insert_time: '1700000000001',
            content: {
              kind: 'text' as const,
              text: n === 100 ? 'Team chat is ready.' : `Message ${n}`,
            },
          };
        });
        reply.result.before = String(end - 49);
      }
      return reply;
    },
  }));
  latest = 200;
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Refresh messages' }),
  );
  await ui.screen.findByText('Message 200');
  assert.equal(ui.screen.queryByText('Team chat is ready.'), null);
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Load older messages' }),
  );
  await ui.screen.findByText('Message 101');
  assert.equal(cursors.at(-1), '151');
});

test('verification warnings survive unrelated pages until retained rows are rechecked', async () => {
  let stage = 0;
  await setup((base) => ({
    ...base,
    chat: async (store, action) => {
      const reply = await base.chat(store, action);
      if (action.action === 'history' && reply.result.kind === 'history') {
        const original = {
          id: '03'.repeat(16),
          sequence: '3',
          sender: null,
          send_time: '1700000000000',
          insert_time: '1700000000001',
          content: { kind: 'text' as const, text: 'Team chat is ready.' },
        };
        const next = {
          id: '04'.repeat(16),
          sequence: '4',
          sender: null,
          send_time: '1700000000000',
          insert_time: '1700000000001',
          content: { kind: 'text' as const, text: 'Next message' },
        };
        reply.result.messages =
          stage === 0 ? [original] : stage === 1 ? [next] : [original, next];
        reply.result.before = stage === 1 ? '4' : '3';
        reply.result.missing_predecessors = stage === 0 ? ['1'] : [];
      }
      return reply;
    },
  }));
  assert.ok(ui.screen.getByText(/incomplete verification/));
  stage = 1;
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Refresh messages' }),
  );
  await ui.screen.findByText('Next message');
  assert.ok(ui.screen.getByText(/incomplete verification/));
  stage = 2;
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Refresh messages' }),
  );
  await ui.waitFor(() =>
    assert.ok(ui.screen.queryByText(/incomplete verification/) === null),
  );
});

for (const offline of [true, false]) {
  test(`pending preparations can be cancelled when channels are ${offline ? 'offline' : 'restricted'}`, async () => {
    let seeded = false;
    await setup(
      (base) => ({
        ...base,
        chat: async (store, action) => {
          if (!seeded) {
            seeded = true;
            await base.chat(store, {
              action: 'prepare-message',
              channel: 'ab'.repeat(16),
              submission: '77'.repeat(16),
              text: 'Cancel this preparation',
            });
          }
          if (offline && action.action === 'sync-inbox')
            throw {
              code: 'offline',
              message: 'Offline',
              fatal: false,
              retryable: true,
              ambiguous: false,
            };
          const reply = await base.chat(store, action);
          if (!offline && reply.result.kind === 'inbox')
            reply.result.channels = reply.result.channels.map((channel) => ({
              ...channel,
              readable: false,
              writable: false,
            }));
          return reply;
        },
      }),
      false,
    );
    await ui.screen.findByText(
      offline ? 'Offline' : 'Your current role cannot read this channel.',
    );
    ui.fireEvent.click(
      ui.screen.getByRole('button', { name: 'Cancel preparation' }),
    );
    await ui.waitFor(() =>
      assert.ok(
        ui.screen.queryByRole('button', { name: 'Cancel preparation' }) ===
          null,
      ),
    );
  });
}

test('durable pending refresh replaces stale prepared state after lost delivery and status replies', async () => {
  let uncertain = false;
  await setup((base) => ({
    ...base,
    chat: async (store, action) => {
      if (action.action === 'attempt') {
        uncertain = true;
        throw {
          code: 'ambiguous',
          message: 'Delivery reply lost',
          fatal: false,
          ambiguous: true,
          retryable: false,
        };
      }
      if (action.action === 'status')
        throw {
          code: 'offline',
          message: 'Status unavailable',
          fatal: false,
          ambiguous: false,
          retryable: true,
        };
      const reply = await base.chat(store, action);
      if (uncertain && reply.result.kind === 'pending')
        reply.result.operations = reply.result.operations.map((op) => ({
          ...op,
          state: 'uncertain',
        }));
      return reply;
    },
  }));
  ui.fireEvent.change(ui.screen.getByRole('textbox', { name: 'Message' }), {
    target: { value: 'recover delivery' },
  });
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Send' }));
  await ui.screen.findByRole('button', { name: 'Check delivery' });
  assert.equal(
    ui.screen.queryByRole('button', { name: 'Cancel preparation' }),
    null,
  );
  assert.equal(
    ui.screen.queryByRole('button', { name: 'Send prepared' }),
    null,
  );
});

test('sidebar receives live unread while Chat is unmounted', async () => {
  let direct!: Bridge;
  let unread = '1';
  await setup(
    (base) => {
      direct = base;
      return {
        ...base,
        chat: async (store, action, view) => {
          const reply = await base.chat(store, action, view);
          if (reply.result.kind === 'inbox')
            reply.result.conversations = reply.result.conversations.map(
              (c) => ({ ...c, unread }),
            );
          return reply;
        },
      };
    },
    false,
    () => {},
    false,
  );
  await ui.screen.findByLabelText('1 unread');
  assert.ok(ui.screen.queryByLabelText('Message history') === null);
  unread = '9007199254740993';
  await direct.chat(
    'team:eng',
    { action: 'mark-read', channel: 'ab'.repeat(16), sequence: '1' },
    'remote',
  );
  await ui.screen.findByLabelText('9007199254740993 unread');
});

test('read-only projection keeps history and removes composer', async () => {
  await setup((base) => ({
    ...base,
    chat: async (store, action, view) => {
      const reply = await base.chat(store, action, view);
      if (reply.result.kind === 'inbox') {
        reply.result.channels = reply.result.channels.map((c) => ({
          ...c,
          writable: false,
        }));
        reply.result.conversations = reply.result.conversations.map((c) => ({
          ...c,
          channel: { ...c.channel, writable: false },
        }));
      }
      return reply;
    },
  }));
  assert.ok(ui.screen.queryByRole('textbox', { name: 'Message' }) === null);
  assert.ok(ui.screen.getByText('Team chat is ready.'));
});

test('uncertain send retains body in thread and merges once after delivery checking', async () => {
  let lost = false;
  let attempts = 0;
  let resolveDelivery = false;
  await setup((base) => ({
    ...base,
    chat: async (store, action, view) => {
      if (action.action === 'attempt') {
        attempts++;
        if (!lost) {
          lost = true;
          throw {
            code: 'ambiguous',
            message: 'Receipt lost',
            fatal: false,
            retryable: false,
            ambiguous: true,
          };
        }
      }
      // Simulate the server finishing the original in-flight request while the
      // client checks status; the UI must never submit a second attempt.
      if (resolveDelivery && action.action === 'status')
        return base.chat(
          store,
          { action: 'attempt', operation: action.operation },
          view,
        );
      const reply = await base.chat(store, action, view);
      if (lost && reply.result.kind === 'pending')
        reply.result.operations = reply.result.operations.map((op) => ({
          ...op,
          state: 'uncertain',
        }));
      if (
        lost &&
        action.action === 'status' &&
        reply.result.kind === 'operation' &&
        reply.result.operation.state === 'prepared'
      )
        reply.result.operation.state = 'uncertain';
      return reply;
    },
  }));
  ui.fireEvent.change(ui.screen.getByRole('textbox', { name: 'Message' }), {
    target: { value: 'Keep this pending body' },
  });
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Send' }));
  await ui.screen.findByRole('button', { name: 'Check delivery' });
  assert.ok(document.querySelector('.chat-messages [data-operation]'));
  assert.equal(ui.screen.getAllByText('Keep this pending body').length, 1);
  resolveDelivery = true;
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Check delivery' }));
  await ui.waitFor(() =>
    assert.ok(
      document.querySelector('.chat-messages [data-operation]') === null,
    ),
  );
  assert.equal(ui.screen.getAllByText('Keep this pending body').length, 1);
  assert.equal(attempts, 1);
});

for (const fault of ['history', 'preview', 'scope'] as const) {
  test(`${fault} integrity containment preserves exactly the appropriate owner`, async () => {
    const bad = 'ab'.repeat(16);
    const healthy = 'ef'.repeat(16);
    let badReads = 0;
    let syncs = 0;
    await setup(
      (base) => ({
        ...base,
        chat: async (store, action, view) => {
          if (action.action === 'history' && action.channel === bad) {
            badReads++;
            if (fault === 'history')
              throw {
                code: 'chat-channel-integrity',
                message: 'Message authentication failed.',
                fatal: true,
                retryable: false,
                ambiguous: false,
              };
          }
          const reply = await base.chat(store, action, view);
          if (action.action === 'sync-inbox' && reply.result.kind === 'inbox') {
            syncs++;
            reply.result.channels = [
              ...reply.result.channels,
              { ...reply.result.channels[0], id: healthy, name: 'healthy' },
            ];
            if (fault === 'preview') {
              reply.result.blocked_channels = [bad];
              reply.result.conversations = reply.result.conversations.map(
                (c) => ({ ...c, preview: null }),
              );
            }
          }
          if (action.action === 'history' && reply.result.kind === 'history') {
            if (action.channel === healthy)
              reply.result.messages = [
                {
                  id: '12'.repeat(16),
                  sequence: '1',
                  sender: null,
                  send_time: '1',
                  insert_time: '1',
                  content: { kind: 'text', text: 'Healthy channel content' },
                },
              ];
            else if (fault === 'scope')
              reply.scope = { ...reply.scope, actor: '01' + 'ed'.repeat(32) };
          }
          return reply;
        },
      }),
      false,
      () => {},
      true,
      true,
    );
    if (fault === 'scope') {
      await ui.screen.findByRole('heading', { name: 'Chat stopped', level: 1 });
      assert.equal(ui.screen.queryByRole('textbox', { name: 'Message' }), null);
      return;
    }
    await ui.screen.findByRole('heading', { name: 'Channel stopped' });
    assert.equal(ui.screen.queryByRole('textbox', { name: 'Message' }), null);
    const reads = badReads;
    ui.fireEvent.click(ui.screen.getByRole('button', { name: /# healthy/ }));
    await ui.screen.findByText('Healthy channel content');
    assert.ok(ui.screen.getByRole('textbox', { name: 'Message' }));
    ui.fireEvent.click(ui.screen.getByRole('button', { name: /# general/ }));
    await ui.screen.findByRole('heading', { name: 'Channel stopped' });
    assert.equal(
      badReads,
      reads,
      'quarantine survives channel navigation without retry',
    );
    assert.ok(syncs > 0);
  });
}
