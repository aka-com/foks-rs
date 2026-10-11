import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, StrictMode, useState } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import { installDom } from './lib/dom-harness';
import { readSource } from './lib/source';
import type { Location } from '../src/location';
import type { Bridge } from '../src/bridge';
import type { ChatAction } from '../src/chat-contract';
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
  incrementalHistory = false,
) {
  const { ChatTeamColumn } = await vite.ssrLoadModule(
    '/src/screens/chat-teams.tsx',
  );
  const { ChatInboxProvider } = await vite.ssrLoadModule(
    '/src/chat/inbox-provider.tsx',
  );
  // The tab is what the shell renders: the team column and, beside it, the
  // conversation of the team the location names.
  const { ChatTab } = await vite.ssrLoadModule('/src/screens/chat-tab.tsx');
  const { OverlayProvider } = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const enabledSnapshot = {
    ...FIXTURE,
    agent: { state: 'ready' as const, historyAfter: incrementalHistory },
    stores: FIXTURE.stores.filter(
      (s) => s.kind !== 'team' || s.id === 'team:eng',
    ),
    servers: FIXTURE.servers.map((s) =>
      s.profileName === 'acme'
        ? { ...s, services: { ...s.services, chat: true } }
        : s,
    ),
  };
  const baseBridge = mockBridge(enabledSnapshot);
  const bridge = override?.(baseBridge) ?? baseBridge;
  const portalRoot = document.getElementById('overlays');
  if (!portalRoot) throw new Error('missing overlay root');
  const overlayRoot = portalRoot;
  let toggleChat = () => {};
  function Host() {
    const [visible, setVisible] = useState(showChat);
    toggleChat = () => setVisible((value) => !value);
    const [location, setLocation] = useState<Location>({
      kind: 'chat',
      ref: 'team:eng',
      channel: 'ab'.repeat(16),
    });
    return createElement(
      StrictMode,
      null,
      createElement(OverlayProvider, {
        backgroundRef: { current: null },
        portalRoot: overlayRoot,
        children: createElement(ChatInboxProvider, {
          bridge,
          snapshot: enabledSnapshot,
          children: visible
            ? createElement(ChatTab, {
                snapshot: enabledSnapshot,
                bridge,
                location,
                onNavigate: (next: Location) => {
                  onNavigate(next);
                  if (navigate) setLocation(next);
                },
              })
            : // The rail no longer lists chats; the Chat tab's team column
              // does, and it carries the unread counts the inbox publishes for
              // teams whose conversation is not mounted.
              createElement(ChatTeamColumn, {
                snapshot: enabledSnapshot,
                onOpen: (ref: string, channel?: string) => {
                  const next: Location = channel
                    ? { kind: 'chat', ref, channel }
                    : { kind: 'chat', ref };
                  onNavigate(next);
                  setLocation(next);
                  setVisible(true);
                },
                onNewChat: () => {},
                onSettings: () => {},
                onTeams: () => {},
              }),
        }),
      }),
    );
  }
  const rendered = ui.render(createElement(Host));
  if (waitForHistory) await ui.screen.findByText('Team chat is ready.');
  return { ...rendered, toggleChat: () => ui.act(() => toggleChat()) };
}
/** A team row's unread badge: the count belongs to the row that names the team. */
async function teamBadge(team: string): Promise<HTMLElement> {
  return ui.waitFor(() => {
    // A team's count is on its channel rows; the heading carries only what
    // the channels cannot say, or their total while folded.
    const badge = [...document.querySelectorAll<HTMLElement>('.chat-team-head')]
      .find((row) => row.querySelector('b')?.textContent === team)
      ?.closest('.chat-team')
      ?.querySelector<HTMLElement>('.chat-channel .chat-unread');
    assert.ok(badge, `${team} carries an unread badge`);
    return badge;
  });
}
/**
 * The create-a-channel form, reached the way the tab offers it: New channel,
 * then the team in the sheet's selector.
 */
async function openChannelSheet() {
  ui.fireEvent.click(
    ui.screen.getAllByRole('button', { name: 'New channel' })[0],
  );
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Team' }));
  ui.fireEvent.click(
    await ui.screen.findByRole('option', { name: /^Engineering/ }),
  );
  await ui.waitFor(() =>
    assert.equal(
      ui.screen.queryByText('Checking saved channel creations…'),
      null,
    ),
  );
  return ui.screen.getByRole('textbox', { name: 'Channel name' });
}
test('composer suppresses IME and repeated Enter, and renders hostile text safely', async () => {
  let preparations = 0;
  let attempts = 0;
  await setup((base) => ({
    ...base,
    chat: async (store, action) => {
      if (action.action === 'submit-message') preparations++;
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
  assert.equal(attempts, 0);
  assert.equal(document.querySelector('.chat-messages img'), null);
  assert.equal((composer as HTMLTextAreaElement).value, '');
  const ownMessage = ui.screen
    .getByText('<img src=x onerror=alert(1)>')
    .closest('.chat-message');
  assert.ok(ownMessage);
  assert.equal(ownMessage.querySelector('.chat-sender')?.textContent, 'You');
  assert.equal(ownMessage.querySelector('.chat-avatar')?.textContent, 'V');
});
test('empty and whitespace-only messages are silent no-ops', async () => {
  let submissions = 0;
  await setup((base) => ({
    ...base,
    chat: async (store, action) => {
      if (action.action === 'submit-message') submissions++;
      return base.chat(store, action);
    },
  }));
  const composer = ui.screen.getByRole('textbox', { name: 'Message' });
  const form = composer.closest('form');
  assert.ok(form);
  for (const value of ['', '   ', '\t\n \u00a0']) {
    ui.fireEvent.change(composer, { target: { value } });
    await ui.act(async () => {
      ui.fireEvent.keyDown(composer, { key: 'Enter' });
      ui.fireEvent.submit(form);
    });
    assert.equal(submissions, 0);
    assert.equal((composer as HTMLTextAreaElement).value, value);
    assert.equal(
      ui.screen.queryByText('The message is empty or exceeds the text limit.'),
      null,
    );
  }
});
test('Option–Enter inserts a newline at the cursor without sending', async () => {
  let submissions = 0;
  await setup((base) => ({
    ...base,
    chat: async (store, action) => {
      if (action.action === 'submit-message') submissions++;
      return base.chat(store, action);
    },
  }));
  const composer = ui.screen.getByRole('textbox', {
    name: 'Message',
  });
  assert.ok(composer instanceof window.HTMLTextAreaElement);
  ui.fireEvent.change(composer, { target: { value: 'hello world' } });
  composer.setSelectionRange(5, 6);
  ui.fireEvent.keyDown(composer, { key: 'Enter', altKey: true });
  assert.equal(composer.value, 'hello\nworld');
  assert.equal(composer.selectionStart, 6);
  assert.equal(submissions, 0);
});

test('the composer exposes only supported actions', async () => {
  await setup();
  for (const label of [
    'Attach a file',
    'Insert an emoji',
    'Set an exploding timer',
  ])
    assert.equal(ui.screen.queryByRole('button', { name: label }), null);
  assert.equal(ui.screen.queryByText('**bold**'), null);
  assert.equal(
    ui.screen.queryByText('Enter to send · Shift+Enter for a new line'),
    null,
  );
  assert.equal(ui.screen.queryByText('@user'), null);
});

test('renders channel descriptions, message timestamps, and channel rows with their last message', async () => {
  await setup();
  // The header shows the description instead of the team and member count.
  await ui.waitFor(() =>
    assert.match(
      document.querySelector('.chat-thread-sub')?.textContent ?? '',
      /^A place for the whole team\.$/,
    ),
  );
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Channel info' }));
  await ui.screen.findAllByText('A place for the whole team.');
  // A channel row is its name, its last message and that message's time, and
  // its unread count; the message itself is drawn once, in the conversation.
  assert.deepEqual(
    [...document.querySelectorAll('.chat-channel .n')].map(
      (row) => row.textContent,
    ),
    ['general'],
  );
  assert.deepEqual(
    [...document.querySelectorAll('.chat-channel-preview')].map(
      (row) => row.textContent,
    ),
    ['Team member: Team chat is ready.'],
  );
  assert.equal(ui.screen.getAllByText('Team chat is ready.').length, 1);
  const time = document.querySelector<HTMLTimeElement>('.chat-message time');
  assert.equal(time?.dateTime, '2023-11-14T22:13:20.001Z');
  assert.match(time?.title ?? '', /Message #1/);
  // The clock alone, with no seconds and no date: the day is carried once by
  // the separator above the run of messages.
  assert.match(time?.textContent ?? '', /^\d{1,2}:\d{2}(\s?(AM|PM))?$/i);
  assert.doesNotMatch(time?.textContent ?? '', /2023/);
  assert.ok(document.querySelector('.chat-daysep')?.textContent);
});

for (const incremental of [false, true])
  test(`conversation inbox wakes and loads ${incremental ? 'incremental' : 'legacy'} history`, async () => {
    const originalFocus = document.hasFocus.bind(document);
    Object.defineProperty(document, 'hasFocus', {
      configurable: true,
      value: () => false,
    });
    let direct!: Bridge['chat'];
    let incoming = false;
    let polls = 0;
    const cursors: (string | undefined)[] = [];
    try {
      await setup(
        (base) => {
          direct = (store, action, view) => base.chat(store, action, view);
          return {
            ...base,
            chat: async (store, action, view) => {
              if (action.action === 'poll-inbox') polls++;
              if (action.action === 'history') cursors.push(action.after);
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
        },
        true,
        () => {},
        true,
        false,
        incremental,
      );
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
      assert.equal(
        cursors.some((cursor) => cursor !== undefined),
        incremental,
      );
      assert.equal(ui.screen.getAllByText('Team chat is ready.').length, 1);
      // Engineering's general channel row carries the count. The label states
      // the count alone: the row it sits in says which channel it belongs to.
      assert.equal(
        (await teamBadge('Engineering')).getAttribute('aria-label'),
        '1 unread',
      );
    } finally {
      Object.defineProperty(document, 'hasFocus', {
        configurable: true,
        value: originalFocus,
      });
    }
  });

test('live sync backs off across repeated post-poll sync failures', async (t) => {
  // Autofocus makes JSDOM's window focused. Keep read acknowledgments (which
  // legitimately invalidate the inbox) out of this background-backoff test.
  t.mock.method(document, 'hasFocus', () => false);
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

test('a read the server refused still invalidates the team', async () => {
  Object.defineProperty(document, 'hasFocus', {
    configurable: true,
    value: () => true,
  });
  const marks: string[] = [];
  const syncs: string[] = [];
  // Counted when the refusal is thrown: the synchronization it causes can
  // land before a poll observes the mark.
  let settled = 0;
  try {
    await setup((base) => ({
      ...base,
      chat: async (store, action, view) => {
        if (action.action === 'sync-inbox') syncs.push(store);
        if (action.action === 'mark-read') {
          marks.push(action.sequence);
          settled = syncs.length;
          throw {
            code: 'io',
            message: 'Connection lost.',
            retryable: true,
            fatal: false,
            ambiguous: false,
          };
        }
        return base.chat(store, action, view);
      },
    }));
    await ui.waitFor(() => assert.equal(marks.length, 1));
    await ui.waitFor(() => assert.ok(syncs.length > settled), {
      timeout: 4_000,
    });
  } finally {
    Object.defineProperty(document, 'hasFocus', {
      configurable: true,
      value: () => false,
    });
  }
});

test('a read mark the server could not take for now is not the thread’s failure to announce', async () => {
  Object.defineProperty(document, 'hasFocus', {
    configurable: true,
    value: () => true,
  });
  const marks: string[] = [];
  try {
    await setup((base) => ({
      ...base,
      chat: async (store, action, view) => {
        if (action.action === 'mark-read') {
          marks.push(action.sequence);
          // The native side staged the read before asking; the next
          // synchronization says it will retry.
          throw {
            code: 'io',
            message: 'Connection lost.',
            retryable: true,
            fatal: false,
            ambiguous: false,
          };
        }
        return base.chat(store, action, view);
      },
    }));
    await ui.waitFor(() => assert.equal(marks.length, 1));
    await ui.act(async () => {});
    assert.equal(ui.screen.queryByText('Connection lost.'), null);
    assert.equal(ui.screen.queryByRole('button', { name: 'Retry' }), null);
  } finally {
    Object.defineProperty(document, 'hasFocus', {
      configurable: true,
      value: () => false,
    });
  }
});

test('lost submit reply recovers its submission without a second send', async () => {
  const submissions: string[] = [];
  let attempts = 0;
  let dropped = false;
  await setup((base) => ({
    ...base,
    chat: async (store, action) => {
      const reply = await base.chat(store, action);
      if (
        action.action === 'submit-message' ||
        action.action === 'prepare-message'
      ) {
        submissions.push(action.submission);
        if (!dropped) {
          dropped = true;
          throw {
            code: 'ambiguous',
            message: 'Submit reply lost',
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
  await ui.screen.findByText('Submit reply lost');
  assert.equal(attempts, 0);
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Check again' }));
  await ui.waitFor(() => assert.equal(submissions.length, 2));
  assert.equal(ui.screen.getAllByText('recover this message').length, 1);
  assert.equal(new Set(submissions).size, 1);
  assert.equal(submissions.length, 2);
  assert.equal(attempts, 0);
});
test('ending the unlocked application during preparation never starts delivery', async () => {
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
      const result = await base.chat(
        store,
        action.action === 'submit-message'
          ? { ...action, action: 'prepare-message' }
          : action,
      );
      if (action.action === 'submit-message') {
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
          message: 'That name is not allowed here.',
          fatal: false,
          retryable: false,
          ambiguous: false,
        };
      return base.chat(store, action);
    },
  }));
  const name = await openChannelSheet();
  ui.fireEvent.change(name, { target: { value: 'refused' } });
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Create channel' }));
  await ui.screen.findByText('That name is not allowed here.');
  assert.equal((name as HTMLInputElement).disabled, false);
  ui.fireEvent.change(name, { target: { value: 'valid' } });
  assert.equal((name as HTMLInputElement).value, 'valid');
  assert.ok(ui.screen.getByRole('dialog', { name: 'Create channel' }));
});

test('ambiguous channel preparation keeps its submission after the sheet closes', async () => {
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
  const name = await openChannelSheet();
  ui.fireEvent.change(name, { target: { value: 'recoverable' } });
  const description = ui.screen.getByRole('textbox', {
    name: 'Channel description',
  });
  ui.fireEvent.change(description, { target: { value: 'Team Decisions' } });
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Create channel' }));
  await ui.screen.findByText('Preparation reply lost');
  assert.equal((description as HTMLInputElement).disabled, true);
  // The application retains the submission when the sheet is dismissed.
  assert.equal(
    ui.screen.getByRole('button', { name: 'Close' }).hasAttribute('disabled'),
    false,
  );
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Close' }));
  assert.equal(ui.screen.queryByRole('dialog'), null);
  // The sheet opens on the open team, Engineering, so the submission saved
  // for it is recovered at once rather than listed for a click.
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'New channel' }));
  ui.fireEvent.click(
    await ui.screen.findByRole('button', { name: 'Retry channel creation' }),
  );
  await ui.screen.findByRole('button', { name: /#recoverable/ });
  await ui.waitFor(() => assert.equal(ui.screen.queryByRole('dialog'), null));
  assert.equal(new Set(submissions).size, 1);
  assert.equal(submissions.length, 2);
  assert.deepEqual(descriptions, ['team decisions', 'team decisions']);
});

test('team members opens the existing membership workflow in the same window', async () => {
  let destination: unknown;
  await setup(undefined, true, (location) => {
    destination = location;
  });
  // Team membership management is accessed via the conversation header's Members button.
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Channel info' }));
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Manage in Teams' }),
  );
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
  const name = await openChannelSheet();
  // Choosing a team returns focus to the custom selector's trigger.
  await ui.waitFor(() =>
    assert.ok(
      document.activeElement ===
        ui.screen.getByRole('button', { name: 'Team' }),
    ),
  );
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
  await ui.screen.findByRole('button', { name: /#design/ });
  await ui.waitFor(() => assert.equal(ui.screen.queryByRole('dialog'), null));
  assert.equal(
    ui.screen.queryByRole('button', { name: 'Finish cleanup' }),
    null,
  );
  assert.equal(ui.screen.queryByText('Needs attention'), null);
  assert.deepEqual(navigations.at(-1), {
    kind: 'chat',
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
      ['chat-message', 'chat-divider', 'chat-message', 'chat-message grouped'],
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

/**
 * An IntersectionObserver whose targets are on screen when `onScreen` says
 * so; `'edge'` is a row twenty times the list's height whose edge is in
 * view. Observing a target reports its state, as a browser's first callback
 * does, and `refresh` reports every target whose state changed.
 */
class TestIntersectionObserver {
  static onScreen: (element: Element) => boolean | 'edge' = () => false;
  static observers = new Set<TestIntersectionObserver>();
  private targets = new Map<Element, boolean | 'edge'>();
  constructor(private callback: IntersectionObserverCallback) {
    TestIntersectionObserver.observers.add(this);
  }
  observe(target: Element) {
    this.targets.set(target, false);
    queueMicrotask(() => this.report([target]));
  }
  unobserve(target: Element) {
    this.targets.delete(target);
  }
  disconnect() {
    this.targets.clear();
    TestIntersectionObserver.observers.delete(this);
  }
  takeRecords() {
    return [];
  }
  private report(targets: Element[]) {
    const entries = targets
      .filter((target) => this.targets.has(target))
      .map((target) => {
        const shown = TestIntersectionObserver.onScreen(target);
        this.targets.set(target, shown);
        return {
          target,
          isIntersecting: shown !== false,
          intersectionRatio: shown === 'edge' ? 0.002 : shown ? 1 : 0,
          intersectionRect: { height: shown === 'edge' ? 16 : shown ? 40 : 0 },
          boundingClientRect: { height: shown === 'edge' ? 8_000 : 40 },
          rootBounds: { height: 400 },
        } as unknown as IntersectionObserverEntry;
      });
    if (entries.length)
      this.callback(entries, this as unknown as IntersectionObserver);
  }
  static refresh() {
    for (const observer of this.observers)
      observer.report(
        [...observer.targets.keys()].filter(
          (target) => observer.targets.get(target) !== this.onScreen(target),
        ),
      );
  }
}
/** Rows whose sequence is in `[from, to]` are on screen; nothing else is. */
function showRows(from: number, to: number) {
  TestIntersectionObserver.onScreen = (element) => {
    const sequence = Number((element as HTMLElement).dataset.sequence ?? NaN);
    return sequence >= from && sequence <= to;
  };
  TestIntersectionObserver.refresh();
}
async function withIntersectionObserver(run: () => Promise<void>) {
  Object.defineProperty(globalThis, 'IntersectionObserver', {
    configurable: true,
    value: TestIntersectionObserver,
  });
  TestIntersectionObserver.onScreen = () => false;
  try {
    await run();
  } finally {
    delete (globalThis as { IntersectionObserver?: unknown })
      .IntersectionObserver;
    TestIntersectionObserver.observers.clear();
  }
}
const sleep = (ms: number) =>
  new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
/**
 * The general channel holding `count` messages, all from another member
 * unless listed in `own`, read through `read`. History follows the agent's
 * range rules: `before` reads the fifty sequences below it and `after` up to
 * fifty rows past it. Setting `state.span` narrows `before` reads to that
 * many sequences, as the agent narrows a range over its limits. `arrive` adds
 * messages and wakes the inbox.
 */
async function busyChannel(count: number, read: number) {
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const other = FIXTURE.parties.find(
    (party) => party.store === 'team:eng' && party.username,
  )?.party_id_hex;
  assert.ok(other);
  const actor = '01' + 'ab'.repeat(32);
  const id = 'ab'.repeat(16);
  const state = {
    count,
    read,
    own: new Set<number>(),
    wakes: 1_000,
    span: 50,
  };
  const history: ChatAction[] = [];
  const marks: string[] = [];
  let base!: Bridge;
  const message = (n: number) => ({
    id: n.toString(16).padStart(32, '0'),
    sequence: String(n),
    sender: state.own.has(n) ? actor : other,
    send_time: '1700000000000',
    insert_time: '1700000000001',
    content: { kind: 'text' as const, text: `Message ${n}` },
  });
  const override = (bridge: Bridge): Bridge => {
    base = bridge;
    return {
      ...bridge,
      chat: async (store, action, view) => {
        if (action.action === 'mark-read') {
          marks.push(action.sequence);
          state.read = Math.max(state.read, Number(action.sequence));
        }
        const reply = await bridge.chat(store, action, view);
        if (action.action === 'history' && reply.result.kind === 'history') {
          history.push(action);
          const rows: number[] = [];
          let gap: boolean | undefined;
          if (action.after !== undefined) {
            const after = Number(action.after);
            gap = state.count < after || state.count - after > 50;
            for (
              let n = Math.max(after + 1, state.count - 49);
              n <= state.count;
              n++
            )
              rows.push(n);
          } else {
            const end =
              action.before === null ? state.count : Number(action.before) - 1;
            for (
              let n = Math.max(1, end - state.span + 1);
              n <= Math.min(end, state.count);
              n++
            )
              rows.push(n);
          }
          reply.result = {
            kind: 'history',
            channel: id,
            messages: (action.after === undefined ? rows : rows.reverse()).map(
              message,
            ),
            before:
              rows.length && Math.min(...rows) > 1
                ? String(Math.min(...rows))
                : null,
            missing_predecessors: [],
            ...(gap === undefined ? {} : { gap }),
          };
        }
        if (reply.result.kind === 'inbox')
          reply.result.conversations = reply.result.conversations.map(
            (conversation) => ({
              ...conversation,
              read_through: String(state.read),
              pending_read: null,
              unread: String(Math.max(0, state.count - state.read)),
            }),
          );
        return reply;
      },
    };
  };
  const arrive = async (senders: ('other' | 'own')[]) => {
    for (const sender of senders) {
      state.count++;
      if (sender === 'own') state.own.add(state.count);
    }
    // A read in the mock's own ledger moves the inbox version, which is
    // what wakes the poll; the conversation it reports is the one above.
    await base.chat('team:eng', {
      action: 'mark-read',
      channel: id,
      sequence: String(state.wakes++),
    });
  };
  return { override, history, marks, state, arrive };
}

test('an unread channel opens at its first unread message and marks only what was on screen', async () => {
  Object.defineProperty(document, 'hasFocus', {
    configurable: true,
    value: () => true,
  });
  const channel = await busyChannel(130, 10);
  try {
    await withIntersectionObserver(async () => {
      await setup(channel.override, false);
      await ui.screen.findByText('Message 50');
      // 120 unread, more than a page: the first request ends ten rows above
      // the first unread message, not at the newest one.
      assert.deepEqual(channel.history[0], {
        action: 'history',
        channel: 'ab'.repeat(16),
        before: '51',
      });
      assert.equal(ui.screen.queryByText('Message 51'), null);
      const rows = [
        ...document.querySelectorAll<HTMLElement>(
          '.chat-divider, .chat-message',
        ),
      ];
      const divider = rows.findIndex((row) =>
        row.classList.contains('chat-divider'),
      );
      assert.equal(rows[divider + 1]?.dataset.sequence, '11');
      // The window ends below the newest message.
      ui.screen.getByRole('button', { name: 'Load newer messages' });
      ui.screen.getByText('80 newer messages');
      ui.screen.getByRole('button', { name: 'Jump to latest' });
      // Nothing on screen yet, so nothing is read.
      await sleep(700);
      assert.deepEqual(channel.marks, []);
      showRows(11, 20);
      await ui.waitFor(() => assert.deepEqual(channel.marks, ['20']));
      // A row that leaves the screen before the dwell is not read.
      showRows(21, 30);
      await sleep(200);
      showRows(0, 0);
      await sleep(600);
      assert.deepEqual(channel.marks, ['20']);
      // Without focus nothing is read, and focus resumes the dwell.
      Object.defineProperty(document, 'hasFocus', {
        configurable: true,
        value: () => false,
      });
      showRows(31, 40);
      await sleep(700);
      assert.deepEqual(channel.marks, ['20']);
      Object.defineProperty(document, 'hasFocus', {
        configurable: true,
        value: () => true,
      });
      window.dispatchEvent(new window.Event('focus'));
      await ui.waitFor(() => assert.deepEqual(channel.marks, ['20', '40']));
      // A row far taller than the list crosses no threshold as it scrolls
      // through; it is measured on scroll once it covers half the list.
      const tall = document.querySelector<HTMLElement>('[data-sequence="41"]');
      assert.ok(tall);
      TestIntersectionObserver.onScreen = (element) =>
        element === tall ? 'edge' : false;
      TestIntersectionObserver.refresh();
      await sleep(700);
      assert.deepEqual(channel.marks, ['20', '40']);
      const scroller = ui.screen.getByLabelText('Message history');
      scroller.getBoundingClientRect = () =>
        ({ top: 0, bottom: 400, height: 400 }) as DOMRect;
      tall.getBoundingClientRect = () =>
        ({ top: -3_000, bottom: 5_000, height: 8_000 }) as DOMRect;
      ui.fireEvent.scroll(scroller);
      await ui.waitFor(() =>
        assert.deepEqual(channel.marks, ['20', '40', '41']),
      );
      // Reaching the foot of the window pages forward until it holds the
      // newest message, with explicit ranges rather than `after` reads.
      TestIntersectionObserver.onScreen = (element) =>
        element.classList.contains('chat-history-newer');
      TestIntersectionObserver.refresh();
      await ui.screen.findByText('Message 130');
      // StrictMode mounts the thread twice, so the anchored read is repeated.
      assert.deepEqual(
        [
          ...new Set(
            channel.history.map((action) =>
              action.action === 'history'
                ? `${action.before}/${action.after}`
                : '',
            ),
          ),
        ],
        ['51/undefined', '101/undefined', '151/undefined'],
      );
      await ui.waitFor(() =>
        assert.equal(
          ui.screen.queryByRole('button', { name: 'Load newer messages' }),
          null,
        ),
      );
    });
  } finally {
    Object.defineProperty(document, 'hasFocus', {
      configurable: true,
      value: () => false,
    });
  }
});

test('an anchored open and forward pages fill ranges the agent narrowed', async () => {
  Object.defineProperty(document, 'hasFocus', {
    configurable: true,
    value: () => false,
  });
  const channel = await busyChannel(130, 10);
  channel.state.span = 25;
  await withIntersectionObserver(async () => {
    await setup(channel.override, false);
    // The anchored range comes back as rows 26 to 50, and the read below it
    // fills the rest, so the page is accepted as one through the owner.
    await ui.screen.findByText('Message 11');
    assert.equal(ui.screen.queryByText(/Conversation closed/), null);
    const rows = [
      ...document.querySelectorAll<HTMLElement>('.chat-divider, .chat-message'),
    ];
    const divider = rows.findIndex((row) =>
      row.classList.contains('chat-divider'),
    );
    assert.equal(rows[divider + 1]?.dataset.sequence, '11');
    assert.equal(rows.at(-1)?.dataset.sequence, '50');
    // Each forward page is filled down to the held tail; the last attaches
    // the window, and focus on its button moves to the list.
    const scroller = ui.screen.getByLabelText('Message history');
    const newer = ui.screen.getByRole('button', {
      name: 'Load newer messages',
    });
    newer.focus();
    ui.fireEvent.click(newer);
    await ui.screen.findByText('Message 100');
    ui.screen.getByText('Message 51');
    await ui.waitFor(() =>
      assert.equal(
        ui.screen
          .getByRole('button', { name: 'Load newer messages' })
          .getAttribute('aria-busy'),
        null,
      ),
    );
    assert.equal(document.activeElement, newer);
    ui.fireEvent.click(newer);
    await ui.screen.findByText('Message 130');
    ui.screen.getByText('Message 101');
    await ui.waitFor(() =>
      assert.equal(
        ui.screen.queryByRole('button', { name: 'Load newer messages' }),
        null,
      ),
    );
    assert.equal(document.activeElement, scroller);
    assert.deepEqual(
      [
        ...new Set(
          channel.history.map((action) =>
            action.action === 'history' ? action.before : '',
          ),
        ),
      ],
      ['51', '26', '101', '76', '151', '126'],
    );
  });
});

test('the jump control counts messages from others that arrived while scrolled up', async () => {
  Object.defineProperty(document, 'hasFocus', {
    configurable: true,
    value: () => false,
  });
  const channel = await busyChannel(5, 5);
  await setup(channel.override, false);
  await ui.screen.findByText('Message 5');
  // Nothing unread: the channel opens at the newest page.
  assert.equal(
    channel.history[0]?.action === 'history' && channel.history[0].before,
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
  await ui.screen.findByRole('button', { name: 'Jump to latest' });
  await channel.arrive(['other', 'own', 'other']);
  await ui.screen.findByText('Message 8');
  const pill = await ui.screen.findByRole('button', {
    name: '2 new messages, jump to latest',
  });
  assert.equal(pill.textContent, '2 new messages');
  ui.fireEvent.click(pill);
  assert.equal(scroller.scrollTop, 1_000);
  await ui.waitFor(() =>
    assert.equal(
      ui.screen.queryByRole('button', { name: /jump to latest/i }),
      null,
    ),
  );
});

for (const via of ['Esc', 'Mark as read'] as const) {
  test(`the new-messages banner and ${via} mark the channel read through its newest message`, async () => {
    Object.defineProperty(document, 'hasFocus', {
      configurable: true,
      value: () => false,
    });
    const channel = await busyChannel(30, 10);
    await setup(channel.override, false);
    await ui.screen.findByText('Message 30');
    // Twenty unread fit the anchored page, so the window is not detached.
    assert.equal(
      channel.history[0]?.action === 'history' && channel.history[0].before,
      '51',
    );
    assert.equal(
      ui.screen.queryByRole('button', { name: 'Load newer messages' }),
      null,
    );
    assert.equal(
      ui.screen.queryByRole('group', { name: 'New messages' }),
      null,
    );
    // Scrolled up, the divider is below the list.
    const scroller = ui.screen.getByLabelText('Message history');
    const divider = ui.screen.getByRole('separator', { name: 'New messages' });
    scroller.getBoundingClientRect = () => ({ top: 0, bottom: 400 }) as DOMRect;
    divider.getBoundingClientRect = () =>
      ({ top: 500, bottom: 520 }) as DOMRect;
    ui.fireEvent.scroll(scroller);
    const banner = await ui.screen.findByRole('group', {
      name: 'New messages',
    });
    assert.match(banner.textContent ?? '', /^20 new messages since /);
    if (via === 'Mark as read') {
      // The button unmounts with the banner, and focus moves to the list.
      const mark = ui
        .within(banner)
        .getByRole('button', { name: /Mark as read/ });
      mark.focus();
      ui.fireEvent.click(mark);
      await ui.waitFor(() => assert.deepEqual(channel.marks, ['30']));
      await ui.waitFor(() =>
        assert.equal(
          ui.screen.queryByRole('group', { name: 'New messages' }),
          null,
        ),
      );
      assert.equal(document.activeElement, scroller);
      return;
    }
    // Esc belongs to a draft in progress.
    const composer = ui.screen.getByRole('textbox', { name: 'Message' });
    ui.fireEvent.change(composer, { target: { value: 'half a thought' } });
    ui.fireEvent.keyDown(composer, { key: 'Escape' });
    await sleep(50);
    assert.deepEqual(channel.marks, []);
    ui.fireEvent.change(composer, { target: { value: '' } });
    // Handled here, so the shell's own Escape does not also act on it.
    assert.equal(ui.fireEvent.keyDown(scroller, { key: 'Escape' }), false);
    await ui.waitFor(() => assert.deepEqual(channel.marks, ['30']));
    await ui.waitFor(() =>
      assert.equal(
        ui.screen.queryByRole('separator', { name: 'New messages' }),
        null,
      ),
    );
    assert.equal(
      ui.screen.queryByRole('group', { name: 'New messages' }),
      null,
    );
    // With nothing unread and no divider, Escape is left to the shell.
    await ui.waitFor(() =>
      assert.equal(ui.fireEvent.keyDown(scroller, { key: 'Escape' }), true),
    );
    assert.deepEqual(channel.marks, ['30']);
  });
}

for (const distinct of [false, true]) {
  test(`chat failures share an inset below the header and deduplicate causes, distinct=${distinct}`, async () => {
    let failing = false;
    const failures = new Set<string>();
    const cause = 'This group could not be read.';
    const { toggleChat: toggle } = await setup((base) => ({
      ...base,
      chat: async (store, action, view) => {
        if (
          failing &&
          ['pending', 'sync-inbox', 'history'].includes(action.action)
        ) {
          failures.add(action.action);
          throw {
            code: 'io',
            message:
              distinct && action.action === 'sync-inbox'
                ? 'Server connection failed.'
                : cause,
            fatal: false,
            retryable: true,
            ambiguous: false,
          };
        }
        return base.chat(store, action, view);
      },
    }));
    // The conversation re-reads its history, its saved work and the team's
    // channels when it is mounted, so closing and reopening it is what makes
    // all three fail at once.
    failing = true;
    toggle();
    toggle();
    await ui.waitFor(() => assert.equal(failures.size, 3));
    const conversation = ui.screen.getByRole('region', {
      name: 'Conversation',
    });
    await ui.waitFor(() =>
      assert.equal(ui.within(conversation).getAllByText(cause).length, 1),
    );
    const header = conversation.querySelector('.chat-thread-header');
    const alerts = conversation.querySelector('.chat-status');
    assert.ok(header && alerts);
    assert.ok(header.nextElementSibling === alerts);
    assert.equal(alerts.querySelectorAll('.band').length, distinct ? 2 : 1);
    assert.equal(alerts.querySelectorAll('.band.stop').length, 1);
    assert.equal(
      ui.within(alerts as HTMLElement).getAllByRole('button', { name: 'Retry' })
        .length,
      distinct ? 2 : 1,
    );
    if (distinct) {
      const warning = ui
        .within(alerts as HTMLElement)
        .getByText('Server connection failed.')
        .closest('.band');
      assert.ok(warning);
      assert.equal(
        warning.parentElement,
        alerts.querySelector('.band.stop')?.parentElement,
      );
    } else {
      assert.equal(
        ui.within(conversation).queryByText('Live updates paused'),
        null,
      );
      failing = false;
      ui.fireEvent.click(
        ui.within(alerts as HTMLElement).getByRole('button', { name: 'Retry' }),
      );
      await ui.waitFor(() =>
        assert.ok(!conversation.querySelector('.chat-status')),
      );
    }
  });
}

test('a team the catalog no longer lists is reported as inaccessible, not as a fault', async () => {
  let failing = false;
  const { toggleChat: toggle } = await setup((base) => ({
    ...base,
    chat: async (store, action, view) => {
      if (failing && action.action === 'history')
        throw {
          code: 'store-not-found',
          message:
            'This group is currently inaccessible. There may have been a server issue or you may have been removed.',
          fatal: false,
          retryable: false,
          ambiguous: false,
        };
      return base.chat(store, action, view);
    },
  }));
  // Remounting the conversation re-reads its history, which is what fails.
  failing = true;
  toggle();
  toggle();
  const conversation = ui.screen.getByRole('region', {
    name: 'Conversation',
  });
  const band = await ui.waitFor(() => {
    const found = conversation.querySelector('.chat-status .band');
    assert.ok(found);
    return found;
  });
  // The band names the team and states the cause as a condition, drawn as
  // information rather than as a critical alert; the agent's own wording is
  // not repeated.
  assert.ok(band.classList.contains('info'));
  assert.ok(!band.classList.contains('stop'));
  assert.equal(band.getAttribute('role'), 'status');
  ui.within(band as HTMLElement).getByText(
    'Engineering is currently inaccessible.',
  );
  ui.within(band as HTMLElement).getByText(
    'There may have been a server issue or you may have been removed.',
  );
  assert.equal(
    ui.within(conversation).queryByText(/no longer in the vault/),
    null,
  );
  ui.within(band as HTMLElement).getByRole('button', { name: 'Retry' });
  failing = false;
  ui.fireEvent.click(
    ui.within(band as HTMLElement).getByRole('button', { name: 'Retry' }),
  );
  await ui.waitFor(() =>
    assert.ok(!conversation.querySelector('.chat-status')),
  );
});

test('deduplicated alerts retain severity and distinct recovery actions without retrying twice', async () => {
  const { ChatAlerts } = await vite.ssrLoadModule('/src/chat/chat-alerts.tsx');
  const calls: string[] = [];
  const retry = { label: 'Retry', run: () => calls.push('inbox') };
  ui.render(
    createElement(ChatAlerts, {
      alerts: [
        {
          message: '  Missing group.  ',
          severity: 'warn',
          label: 'Live updates paused',
          actions: [retry, retry],
        },
        {
          message: 'Missing group.',
          severity: 'crit',
          actions: [
            { label: 'Retry', run: () => calls.push('history') },
            {
              label: 'Retry',
              disabled: true,
              run: () => calls.push('disabled'),
            },
            { label: 'Retry local recovery', run: () => calls.push('local') },
          ],
        },
        {
          message: 'Missing group.',
          severity: 'info',
          label: 'Local message cleanup is pending.',
        },
        { message: ' ', severity: 'warn' },
      ],
    }),
  );
  assert.equal(ui.screen.getAllByRole('alert').length, 1);
  assert.equal(ui.screen.queryByText('Live updates paused'), null);
  assert.equal(
    ui.screen.queryByText('Local message cleanup is pending.'),
    null,
  );
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Retry' }));
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Retry local recovery' }),
  );
  assert.deepEqual(calls, ['inbox', 'history', 'local']);
});

test('chat alert margins are shared across severity and viewport sizes', async () => {
  const css = await readSource('../src/screens/chat.css', import.meta.url);
  assert.match(
    css,
    /\.chat-conversation > \.chat-status\s*\{[^}]*margin: 12px 20px 0;/,
  );
  assert.match(css, /\.chat-status \.band\s*\{[^}]*margin: 0;/);
  assert.match(
    css,
    /@media \(max-width: 860px\)[\s\S]*\.chat-conversation > \.chat-status\s*\{[^}]*margin: 10px 12px 0;/,
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

test('partial synchronization results show an informational note while updates continue', async () => {
  await setup((base) => ({
    ...base,
    chat: async (store, action, view) => {
      const reply = await base.chat(store, action, view);
      if (action.action === 'sync-inbox' && reply.result.kind === 'inbox')
        reply.result.read_retry_pending = true;
      return reply;
    },
  }));
  // The column's row carries the note as a caption; the pane draws it as a
  // note over the conversation, not as a pause of the live updates that go on.
  await ui.screen.findAllByText('Read status will retry.');
  const status = document.querySelector('.chat-status');
  assert.ok(status);
  assert.match(status.textContent ?? '', /Read status will retry\./);
  assert.equal(ui.screen.queryByText('Live updates paused'), null);
});

test('a channel-list failure renders a single retry pane', async () => {
  await setup(
    (base) => ({
      ...base,
      chat: async (store, action, view) => {
        if (action.action === 'sync-inbox')
          throw {
            code: 'chat-access-denied',
            message: 'Your role no longer allows chat in this team.',
            fatal: false,
            retryable: false,
            ambiguous: false,
          };
        return base.chat(store, action, view);
      },
    }),
    false,
  );
  await ui.screen.findByRole('heading', { name: 'Channels unavailable' });
  // The failure is stated once, with one way to try again: neither the
  // "Channel unavailable" pane for the addressed channel nor an invitation to
  // create the first channel is drawn beside it.
  assert.equal(ui.screen.getAllByRole('alert').length, 1);
  assert.match(
    ui.screen.getByRole('alert').textContent ?? '',
    /Your role no longer allows chat/,
  );
  assert.equal(
    ui.screen.queryByRole('heading', { name: 'Channel unavailable' }),
    null,
  );
  assert.equal(ui.screen.queryByText(/No conversations yet/), null);
  assert.equal(ui.screen.getAllByRole('button', { name: 'Retry' }).length, 1);
  assert.equal(ui.screen.queryByRole('button', { name: 'Refresh' }), null);
});

test('prepending older messages preserves scroll position', async () => {
  await setup((base) => ({
    ...base,
    chat: async (store, action) => {
      const reply = await base.chat(store, action);
      // Nothing unread, so the channel opens at its newest page.
      if (reply.result.kind === 'inbox')
        reply.result.conversations = reply.result.conversations.map(
          (conversation) => ({ ...conversation, unread: '0' }),
        );
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

test('a history page with unverified predecessors warns in the conversation', async () => {
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
        reply.result.messages = [original, next];
        reply.result.before = '3';
        reply.result.missing_predecessors = ['1'];
      }
      return reply;
    },
  }));
  assert.ok(ui.screen.getByText(/incomplete verification/));
  // What happens to that warning across later pages is the retained window's
  // own behaviour, covered in chat-history-cache.test.ts.
});

test('an unverifiable message renders as one placeholder row and leaves the channel usable', async () => {
  await setup(
    (base) => ({
      ...base,
      chat: async (store, action) => {
        const reply = await base.chat(store, action);
        if (action.action === 'history' && reply.result.kind === 'history') {
          const row = (sequence: string) => ({
            id: sequence.padStart(2, '0').repeat(16),
            sequence,
            sender: `01${'cd'.repeat(32)}`,
            send_time: '1700000000000',
            insert_time: '1700000000001',
          });
          reply.result.messages = [
            {
              ...row('1'),
              content: { kind: 'text' as const, text: 'Before it' },
            },
            { ...row('2'), content: { kind: 'unverifiable' as const } },
            {
              ...row('3'),
              content: { kind: 'text' as const, text: 'After it' },
            },
            // A team sender is withheld by the agent, and an all-zero ID is
            // keyed by sequence when the reply is decoded.
            {
              ...row('4'),
              id: 'zero:4',
              sender: null,
              content: { kind: 'unverifiable' as const },
            },
          ];
          reply.result.before = null;
        }
        return reply;
      },
    }),
    false,
  );
  const placeholders = await ui.screen.findAllByText(
    'This message could not be decrypted or verified.',
  );
  assert.equal(placeholders.length, 2);
  // The row draws its own header rather than continuing the sender's run.
  const article = placeholders[0].closest('article');
  assert.ok(article);
  assert.equal(article.classList.contains('grouped'), false);
  assert.ok(ui.screen.getByText('Before it'));
  assert.ok(ui.screen.getByText('After it'));
  assert.equal(
    ui.screen.getByRole<HTMLTextAreaElement>('textbox', { name: 'Message' })
      .disabled,
    false,
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
    ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Cancel' }));
    await ui.waitFor(() =>
      assert.ok(ui.screen.queryByRole('button', { name: 'Cancel' }) === null),
    );
  });
}

test('durable pending refresh replaces stale prepared state after lost delivery and status replies', async () => {
  let uncertain = false;
  await setup((base) => ({
    ...base,
    chat: async (store, action) => {
      if (action.action === 'submit-message')
        return base.chat(store, { ...action, action: 'prepare-message' });
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
  await ui.screen.findByRole('button', { name: 'Check again' });
  assert.equal(ui.screen.queryByRole('button', { name: 'Cancel' }), null);
  assert.equal(
    ui.screen.queryByRole('button', { name: 'Send prepared' }),
    null,
  );
});

test('the team column receives live unread while a conversation is unmounted', async () => {
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
  // The collapsed team's badge is the per-team count, on the row that names
  // the team: the label states the count alone, not the team again.
  assert.equal(
    (await teamBadge('Engineering')).getAttribute('aria-label'),
    '1 unread',
  );
  assert.ok(ui.screen.queryByLabelText('Message history') === null);
  unread = '9007199254740993';
  await direct.chat(
    'team:eng',
    { action: 'mark-read', channel: 'ab'.repeat(16), sequence: '1' },
    'remote',
  );
  await ui.waitFor(async () =>
    assert.equal(
      (await teamBadge('Engineering')).getAttribute('aria-label'),
      '9007199254740993 unread',
    ),
  );
});

test('composer autofocuses once per visit and returns on reopening', async () => {
  const page = await setup();
  const composer = ui.screen.getByRole('textbox', { name: 'Message' });
  await ui.waitFor(() => assert.equal(document.activeElement, composer));
  const info = ui.screen.getByRole('button', { name: 'Channel info' });
  info.focus();
  ui.fireEvent.change(composer, { target: { value: 'Draft retained' } });
  assert.equal(document.activeElement, info);
  page.toggleChat();
  page.toggleChat();
  await ui.waitFor(() =>
    assert.equal(
      document.activeElement,
      ui.screen.getByRole('textbox', { name: 'Message' }),
    ),
  );
});

for (const delay of ['sync-inbox', 'load-intent'] as const) {
  for (const interrupted of [false, true]) {
    test(`composer waits for ${delay} and ${interrupted ? 'respects intervening focus' : 'focuses when ready'}`, async () => {
      let release!: () => void;
      const gate = new Promise<void>((resolve) => {
        release = resolve;
      });
      let started = false;
      const page = await setup(
        (base) => ({
          ...base,
          chat: async (store, action, view) => {
            if (action.action === delay) {
              started = true;
              await gate;
            }
            return base.chat(store, action, view);
          },
        }),
        false,
      );
      const other = document.createElement('input');
      document.body.append(other);
      try {
        await ui.waitFor(() => assert.ok(started));
        const composer = ui.screen.queryByRole('textbox', { name: 'Message' });
        if (composer) assert.notEqual(document.activeElement, composer);
        if (interrupted) other.focus();
        await ui.act(async () => release());
        await ui.screen.findByText('Team chat is ready.');
        const ready = await ui.screen.findByRole('textbox', {
          name: 'Message',
        });
        ui.fireEvent.change(ready, { target: { value: 'Ready' } });
        await ui.waitFor(() =>
          assert.equal(
            ui.screen.getByRole<HTMLButtonElement>('button', {
              name: 'Send',
            }).disabled,
            false,
          ),
        );
        assert.equal(document.activeElement, interrupted ? other : ready);
      } finally {
        release();
        page.unmount();
        other.remove();
      }
    });
  }
}

for (const permission of ['readable', 'writable'] as const) {
  test(`composer autofocus waits for ${permission} permission from live sync`, async () => {
    let allowed = false;
    let direct!: Bridge['chat'];
    await setup((base) => {
      direct = (store, action, view) => base.chat(store, action, view);
      return {
        ...base,
        chat: async (store, action, view) => {
          const reply = await base.chat(store, action, view);
          if (reply.result.kind === 'inbox') {
            reply.result.channels = reply.result.channels.map((c) => ({
              ...c,
              [permission]: allowed,
            }));
            reply.result.conversations = reply.result.conversations.map(
              (c) => ({
                ...c,
                channel: { ...c.channel, [permission]: allowed },
              }),
            );
          }
          return reply;
        },
      };
    }, false);
    await ui.screen.findByText(
      permission === 'readable'
        ? 'Read access required'
        : 'Your current role can read this channel but cannot send messages.',
    );
    assert.equal(ui.screen.queryByRole('textbox', { name: 'Message' }), null);
    allowed = true;
    const prepared = await direct('team:eng', {
      action: 'prepare-message',
      submission: '89'.repeat(16),
      channel: 'ab'.repeat(16),
      text: 'Permission changed',
    });
    if (prepared.result.kind !== 'operation')
      throw new Error('missing operation');
    await direct('team:eng', {
      action: 'attempt',
      operation: prepared.result.operation.id,
    });
    const composer = await ui.screen.findByRole('textbox', { name: 'Message' });
    await ui.waitFor(() => assert.equal(document.activeElement, composer));
  });
}

for (const interrupt of [
  'navigation',
  'modal',
  'pointer',
  'keyboard',
] as const) {
  test(`pending composer autofocus respects ${interrupt}`, async () => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    let started = false;
    const page = await setup(
      (base) => ({
        ...base,
        chat: async (store, action, view) => {
          if (action.action === 'load-intent') {
            started = true;
            await gate;
          }
          return base.chat(store, action, view);
        },
      }),
      false,
    );
    const modal = document.createElement('div');
    try {
      await ui.waitFor(() => assert.ok(started));
      if (interrupt === 'navigation') page.toggleChat();
      if (interrupt === 'modal') {
        modal.setAttribute('aria-modal', 'true');
        document.body.append(modal);
      }
      if (interrupt === 'pointer') ui.fireEvent.pointerDown(document.body);
      if (interrupt === 'keyboard')
        ui.fireEvent.keyDown(document.body, { key: 'Tab' });
      const focused = document.activeElement;
      await ui.act(async () => release());
      if (interrupt !== 'navigation') {
        const composer = await ui.screen.findByRole('textbox', {
          name: 'Message',
        });
        ui.fireEvent.change(composer, { target: { value: 'Ready' } });
        await ui.waitFor(() =>
          assert.equal(
            ui.screen.getByRole<HTMLButtonElement>('button', { name: 'Send' })
              .disabled,
            false,
          ),
        );
      }
      assert.equal(document.activeElement, focused);
    } finally {
      release();
      page.unmount();
      modal.remove();
    }
  });
}

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
      if (action.action === 'submit-message')
        return base.chat(store, { ...action, action: 'prepare-message' }, view);
      if (action.action === 'attempt') {
        attempts++;
        if (!lost) {
          lost = true;
          throw {
            code: 'ambiguous',
            message: 'Confirmation lost',
            fatal: false,
            retryable: false,
            ambiguous: true,
          };
        }
      }
      // Simulate the server finishing the original in-flight request while the
      // client checks status; the UI must never submit a second attempt.
      if (resolveDelivery && action.action === 'reconcile')
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
  await ui.screen.findByRole('button', { name: 'Check again' });
  assert.ok(document.querySelector('.chat-messages [data-operation]'));
  assert.equal(ui.screen.getAllByText('Keep this pending body').length, 1);
  resolveDelivery = true;
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Check again' }));
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
      // The blocked notice replaces the conversation; the team column stays.
      await ui.screen.findByRole('heading', { name: 'Chat stopped' });
      assert.equal(ui.screen.queryByRole('textbox', { name: 'Message' }), null);
      return;
    }
    await ui.screen.findByRole('heading', { name: 'Channel stopped' });
    assert.equal(ui.screen.queryByRole('textbox', { name: 'Message' }), null);
    const reads = badReads;
    ui.fireEvent.click(ui.screen.getByRole('button', { name: /#healthy/ }));
    await ui.screen.findByText('Healthy channel content');
    assert.ok(ui.screen.getByRole('textbox', { name: 'Message' }));
    ui.fireEvent.click(ui.screen.getByRole('button', { name: /#general/ }));
    await ui.screen.findByRole('heading', { name: 'Channel stopped' });
    assert.equal(
      badReads,
      reads,
      'quarantine survives channel navigation without retry',
    );
    assert.ok(syncs > 0);
  });
}

test('opening chat from a populated shell establishes history ownership before child effects', async () => {
  await setup(
    (base) => ({
      ...base,
      chat: async (store, action, view) => {
        const reply = await base.chat(store, action, view);
        if (reply.result.kind === 'inbox')
          reply.result.conversations = reply.result.conversations.map((c) => ({
            ...c,
            unread: '1',
          }));
        return reply;
      },
    }),
    false,
    () => {},
    false,
  );
  // The unread badge proves that the inbox snapshot populated before mount.
  assert.equal(
    (await teamBadge('Engineering')).getAttribute('aria-label'),
    '1 unread',
  );
  // Opening a channel is a click on the channel's own row under the heading.
  const heading = [
    ...document.querySelectorAll<HTMLElement>('.chat-team-head'),
  ].find((row) => row.querySelector('b')?.textContent === 'Engineering');
  const general = heading
    ?.closest('.chat-team')
    ?.querySelector<HTMLElement>('.chat-channel');
  assert.ok(general);
  ui.fireEvent.click(general);
  await ui.screen.findByText('Team chat is ready.');
  assert.equal(ui.screen.queryByText('Conversation closed.'), null);
});

test('initial history loading is visible and never claims the conversation is empty', async () => {
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  try {
    await setup(
      (base) => ({
        ...base,
        chat: async (store, action, view) => {
          if (action.action === 'history') await gate;
          return base.chat(store, action, view);
        },
      }),
      false,
    );
    await ui.screen.findByRole('status', { name: 'Loading messages' });
    assert.equal(
      ui.screen.queryByText(
        'No messages yet. Send a message to start the conversation.',
      ),
      null,
    );
    release();
    await ui.screen.findByText('Team chat is ready.');
    assert.equal(
      ui.screen.queryByRole('status', { name: 'Loading messages' }),
      null,
    );
  } finally {
    release();
  }
});

test('reopening a conversation retains verified messages during a delayed or failed refresh', async () => {
  let delay = false;
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  try {
    const view = await setup((base) => ({
      ...base,
      chat: async (store, action, owner) => {
        if (action.action === 'history' && delay) {
          await gate;
          throw {
            code: 'io',
            message: 'History refresh failed',
            retryable: true,
            fatal: false,
            ambiguous: false,
          };
        }
        return base.chat(store, action, owner);
      },
    }));
    view.toggleChat();
    delay = true;
    view.toggleChat();
    assert.ok(
      document
        .querySelector('.chat-message')
        ?.textContent?.includes('Team chat is ready.'),
    );
    assert.equal(
      ui.screen.queryByRole('status', { name: 'Loading messages' }),
      null,
    );
    release();
    await ui.screen.findByRole('alert');
    assert.ok(
      document
        .querySelector('.chat-message')
        ?.textContent?.includes('Team chat is ready.'),
    );
  } finally {
    release();
  }
});

test('channel info keeps channel overrides and links to device notification settings', async (t) => {
  const destinations: unknown[] = [];
  await setup(undefined, true, (location) => destinations.push(location));
  // eslint-disable-next-line @typescript-eslint/unbound-method
  const originalFocus = HTMLElement.prototype.focus;
  const panelFocus: (FocusOptions | undefined)[] = [];
  t.mock.method(
    HTMLElement.prototype,
    'focus',
    function (this: HTMLElement, options?: FocusOptions) {
      if (this.classList.contains('chat-info')) panelFocus.push(options);
      originalFocus.call(this, options);
    },
  );
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Channel info' }));
  const panel = await ui.screen.findByRole('complementary', {
    name: 'Channel info',
  });
  assert.equal(document.activeElement, panel);
  assert.ok(panelFocus.length > 0);
  assert.ok(panelFocus.every((options) => options?.preventScroll === true));
  ui.fireEvent.keyDown(panel, { key: 'Escape' });
  assert.equal(
    ui.screen.queryByRole('complementary', { name: 'Channel info' }),
    null,
  );
  assert.equal(
    document.activeElement,
    ui.screen.getByRole('button', { name: 'Channel info' }),
  );
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Channel info' }));
  assert.equal(
    ui.screen.queryByRole('checkbox', {
      name: 'Enable desktop alerts on this device',
    }),
    null,
  );
  await ui.screen.findByLabelText('Alerts');
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Device notification settings' }),
  );
  assert.deepEqual(destinations.at(-1), {
    kind: 'settings',
    section: 'preferences',
  });
});

class ManualClock {
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
  async advance(milliseconds: number) {
    const end = this.time + milliseconds;
    for (;;) {
      const next = [...this.tasks].sort((a, b) => a[1].due - b[1].due)[0];
      if (!next || next[1].due > end) break;
      this.time = next[1].due;
      this.tasks.delete(next[0]);
      await ui.act(async () => {
        next[1].fn();
        for (let i = 0; i < 50; i++) await Promise.resolve();
      });
    }
    this.time = end;
  }
}

test('the provider stops periodic team synchronization while the window is hidden', async () => {
  const { ChatInboxProvider } = await vite.ssrLoadModule(
    '/src/chat/inbox-provider.tsx',
  );
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const snapshot = {
    ...FIXTURE,
    stores: FIXTURE.stores.filter(
      (store) => store.kind !== 'team' || store.id === 'team:eng',
    ),
    servers: FIXTURE.servers.map((server) =>
      server.profileName === 'acme'
        ? {
            ...server,
            compatibility: { status: 'not-required' as const },
            services: { ...server.services, chat: true },
          }
        : server,
    ),
  };
  const base = mockBridge(snapshot);
  const syncs: string[] = [];
  const bridge: Bridge = {
    ...base,
    chat: async (store, action, view) => {
      if (action.action === 'sync-inbox') syncs.push(store);
      // The poll is answered by nothing in this test: it is the round trip
      // that stays open while the window is hidden.
      if (action.action === 'poll-inbox') return new Promise(() => {});
      return base.chat(store, action, view);
    },
  };
  const visibility = (state: 'visible' | 'hidden') => {
    Object.defineProperty(document, 'visibilityState', {
      configurable: true,
      value: state,
    });
    Object.defineProperty(document, 'hidden', {
      configurable: true,
      value: state === 'hidden',
    });
  };
  visibility('visible');
  const clock = new ManualClock();
  try {
    ui.render(
      createElement(ChatInboxProvider, {
        bridge,
        snapshot,
        clock,
        children: null,
      }),
    );
    await clock.advance(1_000);
    assert.deepEqual(syncs, ['team:eng']);
    // An idle team whose poll is neither failing nor degraded waits minutes,
    // not seconds, for its periodic pass.
    await clock.advance(30_000);
    assert.equal(syncs.length, 1);
    await clock.advance(300_000);
    assert.equal(syncs.length, 2);
    visibility('hidden');
    await ui.act(async () => {
      document.dispatchEvent(new Event('visibilitychange'));
    });
    await clock.advance(600_000);
    assert.equal(syncs.length, 2);
    visibility('visible');
    await ui.act(async () => {
      document.dispatchEvent(new Event('visibilitychange'));
    });
    await clock.advance(1_000);
    assert.equal(syncs.length, 3);
  } finally {
    visibility('visible');
  }
});

test('clicking outside channel info dismisses it without stealing focus or toggling it back open', async () => {
  await setup();
  const toggle = ui.screen.getByRole('button', { name: 'Channel info' });
  ui.fireEvent.click(toggle);
  const panel = await ui.screen.findByRole('complementary', {
    name: 'Channel info',
  });
  const pointer = (target: Element) =>
    ui.fireEvent(
      target,
      new window.MouseEvent('pointerdown', { bubbles: true, button: 0 }),
    );
  pointer(panel);
  assert.ok(ui.screen.queryByRole('complementary', { name: 'Channel info' }));
  const composer = ui.screen.getByRole('textbox', { name: 'Message' });
  composer.focus();
  pointer(composer);
  assert.equal(
    ui.screen.queryByRole('complementary', { name: 'Channel info' }),
    null,
  );
  assert.equal(document.activeElement, composer);
  ui.fireEvent.click(toggle);
  pointer(toggle);
  ui.fireEvent.click(toggle);
  assert.equal(
    ui.screen.queryByRole('complementary', { name: 'Channel info' }),
    null,
  );
});
