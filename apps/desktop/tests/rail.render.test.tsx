/**
 * The navigation rail: six tabs, the summed chat badge, the attention dot,
 * Control-Tab, and the account menu's own commands.
 *
 * The rail is rendered directly rather than through the shell so the inbox and
 * the lock command can be driven from the test.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
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
test.after(async () => vite.close());

interface Journal {
  navigations: Location[];
  locks: number;
}

/** Renders the rail over the fixture, with every team's chat reporting `unread`. */
async function rail(
  location: Location = { kind: 'all' },
  unread = '0',
  attention = 0,
  collapsed = false,
) {
  const { Sidebar } = await vite.ssrLoadModule('/src/shell/sidebar.tsx');
  const { ChatInboxProvider } = await vite.ssrLoadModule(
    '/src/chat/inbox-provider.tsx',
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
  // Chat on every fixture server, so the badge sums more than one team.
  const snapshot = {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server) => ({
      ...server,
      capabilities: { ...server.capabilities, chat: true },
    })),
  };
  const base: Bridge = mockBridge(snapshot);
  const bridge: Bridge = {
    ...base,
    chat: async (store, action, view) => {
      const reply = await base.chat(store, action, view);
      if (reply.result.kind === 'inbox')
        reply.result.conversations = reply.result.conversations.map(
          (conversation) => ({ ...conversation, unread }),
        );
      return reply;
    },
  };
  const journal: Journal = { navigations: [], locks: 0 };
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  const rendered = ui.render(
    createElement(OverlayProvider, {
      backgroundRef: { current: null },
      portalRoot,
      children: createElement(ChatInboxProvider, {
        bridge,
        snapshot,
        children: createElement(Sidebar, {
          snapshot,
          location,
          attention,
          onNavigate: (next: Location) => journal.navigations.push(next),
          onLock: () => {
            journal.locks += 1;
          },
          collapsed,
          pinned: false,
          onToggleCollapsed: () => {},
        }),
      }),
    }),
  );
  return { rendered, journal, snapshot };
}

function tabs(): HTMLButtonElement[] {
  return [
    ...document.querySelectorAll<HTMLButtonElement>(
      '.side.rail .rail-tabs .nav',
    ),
  ];
}

test('the rail draws six tabs and marks the one that owns the location', async () => {
  await rail({ kind: 'group-settings', ref: 'team:eng' });
  assert.deepEqual(
    tabs().map((tab) => tab.querySelector('.t')?.textContent),
    ['People', 'Chat', 'Files', 'Teams', 'Devices', 'Settings'],
  );
  // A group's settings page belongs to Teams.
  assert.equal(tabs()[3].getAttribute('aria-current'), 'page');
  assert.equal(tabs().filter((tab) => tab.className.includes('on')).length, 1);
});

test('Chat carries the summed unread and People the attention dot', async () => {
  const { rendered } = await rail({ kind: 'all' }, '2', 3);
  // The badge is one number over every team whose chat this Mac can read: the
  // fixture's two readable groups, two unread apiece.
  const badge = await ui.waitFor(() => {
    const node = document.querySelector('.side.rail .chat-unread');
    assert.ok(node, 'the Chat tab draws its badge');
    assert.notEqual(node.textContent, '…');
    return node;
  });
  assert.equal(badge.textContent, '4');
  assert.equal(badge.getAttribute('aria-label'), '4 unread');
  assert.ok(tabs()[0].querySelector('.dot'), 'People carries the dot');
  assert.equal(rendered.container.querySelectorAll('.nav .dot').length, 1);
});

test('no unread and nothing to attend to leaves both marks off', async () => {
  await rail({ kind: 'all' }, '0', 0);
  await ui.waitFor(() => {
    assert.ok(document.querySelector('.side.rail .rail-tabs .nav'));
  });
  assert.equal(document.querySelector('.side.rail .chat-unread'), null);
  assert.equal(document.querySelector('.side.rail .nav .dot'), null);
});

test('a tab click and Control-Tab both navigate over the six tabs', async () => {
  const { journal } = await rail({ kind: 'files' });
  ui.fireEvent.click(tabs()[0]);
  assert.deepEqual(journal.navigations.at(-1), { kind: 'people' });

  // Files is the third tab, so forward is Teams and backward is Chat.
  ui.fireEvent.keyDown(document, { key: 'Tab', ctrlKey: true });
  assert.deepEqual(journal.navigations.at(-1), { kind: 'teams' });
  ui.fireEvent.keyDown(document, {
    key: 'Tab',
    ctrlKey: true,
    shiftKey: true,
  });
  assert.deepEqual(journal.navigations.at(-1), { kind: 'chat' });
  // A plain Tab is the browser's own; the rail ignores it.
  const before = journal.navigations.length;
  ui.fireEvent.keyDown(document, { key: 'Tab' });
  assert.equal(journal.navigations.length, before);
});

test('a tab is titled only where its label is hidden', async () => {
  await rail({ kind: 'files' });
  assert.deepEqual(
    tabs().map((tab) => tab.getAttribute('title')),
    [null, null, null, null, null, null],
  );
  ui.cleanup();
  await rail({ kind: 'files' }, '0', 0, true);
  assert.deepEqual(
    tabs().map((tab) => tab.getAttribute('title')),
    ['People', 'Chat', 'Files', 'Teams', 'Devices', 'Settings'],
  );
});

test('the cycle helpers hold the rail order and wrap at both ends', async () => {
  const { sidebarCycleLocations, nextSidebarCycleLocation } =
    (await vite.ssrLoadModule(
      '/src/shell/sidebar.tsx',
    )) as typeof import('../src/shell/sidebar');
  assert.deepEqual(sidebarCycleLocations(), [
    { kind: 'people' },
    { kind: 'chat' },
    { kind: 'files' },
    { kind: 'teams' },
    { kind: 'devices' },
    { kind: 'settings' },
  ]);
  // Control-Tab walks the tab a location belongs to, not the location itself:
  // a group's settings page is on Teams, so forward from it is Devices.
  assert.deepEqual(
    nextSidebarCycleLocation({ kind: 'group-settings', ref: 'team:eng' }, 1),
    { kind: 'devices' },
  );
  assert.deepEqual(nextSidebarCycleLocation({ kind: 'store', ref: 'x' }, -1), {
    kind: 'chat',
  });
  // Both ends wrap.
  assert.deepEqual(nextSidebarCycleLocation({ kind: 'settings' }, 1), {
    kind: 'people',
  });
  assert.deepEqual(nextSidebarCycleLocation({ kind: 'people' }, -1), {
    kind: 'settings',
  });
  // First run belongs to no tab, so the walk starts at the end it came from.
  assert.deepEqual(
    nextSidebarCycleLocation({ kind: 'first-run', step: 'who' }, 1),
    { kind: 'people' },
  );
  assert.deepEqual(
    nextSidebarCycleLocation({ kind: 'first-run', step: 'who' }, -1),
    { kind: 'settings' },
  );
});

test('the account menu switches account, adds one, and locks the app', async () => {
  const { journal } = await rail({ kind: 'people', store: 'acct:personal' });
  const header = document.querySelector<HTMLButtonElement>('.side.rail .who');
  assert.ok(header);
  assert.equal(header.querySelector('.t b')?.textContent, 'satoshi');

  const open = async (): Promise<HTMLElement> => {
    ui.fireEvent.click(header);
    return ui.waitFor(() => {
      const node = document.querySelector<HTMLElement>('[role="menu"]');
      assert.ok(node);
      return node;
    });
  };

  // Choosing an account keeps the page and changes whose account it acts on.
  let menu = await open();
  const other = [...menu.querySelectorAll('button')].find((button) =>
    button.textContent?.includes('vitalik'),
  );
  assert.ok(other, 'the menu lists the second fixture account');
  ui.fireEvent.click(other);
  assert.deepEqual(journal.navigations.at(-1), {
    kind: 'people',
    store: 'acct:work',
  });

  menu = await open();
  const add = [...menu.querySelectorAll('button')].find(
    (button) => button.textContent === 'Add an account or server…',
  );
  assert.ok(add);
  ui.fireEvent.click(add);
  assert.deepEqual(journal.navigations.at(-1), {
    kind: 'first-run',
    step: 'who',
  });

  menu = await open();
  const lock = [...menu.querySelectorAll('button')].find(
    (button) => button.textContent === 'Lock',
  );
  assert.ok(lock);
  ui.fireEvent.click(lock);
  assert.equal(journal.locks, 1);
});

test('closing the account menu with the mouse leaves the header unfocused', async () => {
  await rail({ kind: 'people', store: 'acct:personal' });
  const header = document.querySelector<HTMLButtonElement>('.side.rail .who');
  assert.ok(header);

  // A mouse click reports `detail > 0`. The menu hands focus back to its
  // anchor when it closes; a focused header would hold a collapsed rail open
  // through `:focus-within`, so the rail blurs it again.
  ui.fireEvent.click(header, { detail: 1 });
  const menu = await ui.waitFor(() => {
    const node = document.querySelector<HTMLElement>('[role="menu"]');
    assert.ok(node);
    return node;
  });
  const lock = [...menu.querySelectorAll('button')].find(
    (button) => button.textContent === 'Lock',
  );
  assert.ok(lock);
  ui.fireEvent.click(lock, { detail: 1 });
  // Two microtask turns: the menu's own restore, then the rail's blur.
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
  assert.notEqual(document.activeElement, header);

  // Keyboard activation reports `detail === 0` and keeps focus where the
  // reader left it, so the rail stays open for the next key.
  ui.fireEvent.click(header);
  const again = await ui.waitFor(() => {
    const node = document.querySelector<HTMLElement>('[role="menu"]');
    assert.ok(node);
    return node;
  });
  const escape = [...again.querySelectorAll('button')].find(
    (button) => button.textContent === 'Lock',
  );
  assert.ok(escape);
  ui.fireEvent.click(escape);
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(document.activeElement, header);
});
