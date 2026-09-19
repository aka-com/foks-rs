/**
 * The navigation rail: six tabs, the summed chat badge, the attention dot on
 * the account avatar, Control-Tab, and the account menu's own commands.
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
  unread: string | ((base: Bridge) => Bridge['chat']) = '0',
  attention = 0,
  collapsed = false,
  /** Mount a modal dialog beside the rail, as a sheet or the palette would. */
  dialog = false,
  teamRequests: { label: string; description: string } | null = null,
  devicesAlert: { description: string } | null = null,
  settingsAlert: { description: string } | null = null,
  agent: import('../src/shell/sidebar').RailAgentState = 'ready',
) {
  const { Sidebar } = await vite.ssrLoadModule('/src/shell/sidebar.tsx');
  const { ChatInboxProvider } = await vite.ssrLoadModule(
    '/src/chat/inbox-provider.tsx',
  );
  const { Dialog, OverlayProvider } = (await vite.ssrLoadModule(
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
      services: { ...server.services, chat: true },
    })),
  };
  const base: Bridge = mockBridge(snapshot);
  const bridge: Bridge = {
    ...base,
    chat:
      typeof unread === 'function'
        ? unread(base)
        : async (store, action, view) => {
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
      children: [
        createElement(ChatInboxProvider, {
          key: 'inbox',
          bridge,
          snapshot,
          children: createElement(Sidebar, {
            snapshot,
            location,
            attention,
            teamRequests,
            devicesAlert,
            settingsAlert,
            agent,
            onNavigate: (next: Location) => journal.navigations.push(next),
            onLock: () => {
              journal.locks += 1;
            },
            collapsed,
          }),
        }),
        dialog
          ? createElement(Dialog, { key: 'dialog', children: 'A sheet' })
          : null,
      ],
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
    ['Files', 'Chat', 'Teams', 'Devices', 'Account', 'Settings'],
  );
  // A group's settings page belongs to Teams.
  assert.equal(tabs()[2].getAttribute('aria-current'), 'page');
  assert.equal(tabs().filter((tab) => tab.className.includes('on')).length, 1);
});

test('Chat displays total unread and the avatar displays an attention indicator', async () => {
  const { journal } = await rail({ kind: 'all' }, '2', 3);
  // While unread counts load, Chat displays a status spinner with an
  // accessible label.
  const spinner = document.querySelector('.side.rail .rail-tail.loading');
  assert.ok(spinner, 'the Chat tab spins while its counts load');
  assert.equal(spinner.getAttribute('role'), 'status');
  assert.equal(spinner.getAttribute('aria-label'), 'Loading unread counts');
  assert.ok(spinner.querySelector('.spin'));
  // The badge is one number over every team whose chat this Mac can read: the
  // fixture's two readable groups, two unread apiece.
  const badge = await ui.waitFor(() => {
    const node = document.querySelector('.side.rail .rail-tail.count');
    assert.ok(node, 'the Chat tab draws its count');
    return node;
  });
  assert.equal(document.querySelector('.side.rail .rail-tail.loading'), null);
  assert.equal(badge.textContent, '4');
  assert.equal(badge.getAttribute('aria-label'), '4 unread');
  // Attention is advertised on the account avatar, not on a tab, and the dot
  // is the control that opens the list.
  const dot = document.querySelector<HTMLButtonElement>('.side.rail .attn');
  assert.ok(dot, 'the avatar carries the dot');
  assert.equal(dot.getAttribute('aria-label'), 'Needs attention');
  assert.equal(document.querySelector('.rail-tabs .dot'), null);
  ui.fireEvent.click(dot);
  assert.deepEqual(journal.navigations.at(-1), { kind: 'people' });
});

test('no unread and nothing to attend to leaves both marks off', async () => {
  await rail({ kind: 'all' }, '0', 0);
  await ui.waitFor(() => {
    assert.ok(document.querySelector('.side.rail .rail-tabs .nav'));
  });
  await ui.waitFor(() => {
    assert.equal(document.querySelector('.side.rail .rail-tail.loading'), null);
  });
  assert.equal(document.querySelector('.side.rail .rail-tail'), null);
  assert.equal(document.querySelector('.side.rail .attn'), null);
});

for (const collapsed of [false, true]) {
  for (const state of ['failed', 'degraded'] as const) {
    test(`Chat shows a warning instead of loading for ${state} counts, collapsed=${collapsed}`, async () => {
      await rail(
        { kind: 'all' },
        (base) => async (store, action, view) => {
          if (state === 'failed')
            throw {
              code: 'profile-busy',
              message: 'The inbox could not be read.',
              retryable: true,
              fatal: false,
              ambiguous: false,
            };
          const reply = await base.chat(store, action, view);
          if (reply.result.kind === 'inbox') {
            reply.result.degraded = true;
            reply.result.conversations = reply.result.conversations.map(
              (conversation) => ({ ...conversation, unread: '0' }),
            );
          }
          return reply;
        },
        0,
        collapsed,
      );
      await ui.waitFor(() => {
        const chat = tabs()[1];
        assert.ok(!chat.querySelector('.rail-tail.loading'));
        const warning = chat.querySelector('.rail-tail.dot.warn');
        assert.ok(warning);
        assert.match(
          warning.getAttribute('aria-label') ?? '',
          state === 'failed' ? /unavailable/i : /incomplete/i,
        );
      });
    });
  }
}

test('Chat does not hide a failed team behind another team’s unread count', async () => {
  await rail({ kind: 'all' }, (base) => async (store, action, view) => {
    if (store === 'team:eng')
      throw {
        code: 'profile-busy',
        message: 'The inbox could not be read.',
        retryable: true,
        fatal: false,
        ambiguous: false,
      };
    const reply = await base.chat(store, action, view);
    if (reply.result.kind === 'inbox')
      reply.result.conversations = reply.result.conversations.map(
        (conversation) => ({ ...conversation, unread: '2' }),
      );
    return reply;
  });
  await ui.waitFor(() => {
    const warning = tabs()[1].querySelector('.rail-tail.dot.warn');
    assert.ok(warning);
    assert.match(warning.getAttribute('aria-label') ?? '', /2 known unread/);
    assert.match(warning.getAttribute('aria-label') ?? '', /unavailable/i);
    assert.equal(tabs()[1].querySelector('.rail-tail.loading'), null);
  });
});

test('renders a Teams count, Devices and Settings dots, and no empty indicators', async () => {
  await rail(
    { kind: 'all' },
    '0',
    0,
    false,
    false,
    { label: '2', description: '2 requests to join a team' },
    { description: 'An account has no paper key' },
    { description: 'foks.partner.dev: not verified' },
  );
  const [, , teamsTab, devicesTab, , settingsTab] = tabs();
  const teamsBadge = teamsTab.querySelector('.rail-tail.count');
  assert.ok(teamsBadge, 'Teams carries a count in the same slot as Chat');
  assert.equal(teamsBadge.textContent, '2');
  assert.equal(
    teamsBadge.getAttribute('aria-label'),
    '2 requests to join a team',
  );
  const devicesDot = devicesTab.querySelector('.rail-tail.dot');
  assert.ok(devicesDot, 'Devices carries its dot');
  assert.equal(devicesDot.classList.contains('warn'), false);
  assert.equal(
    devicesDot.getAttribute('aria-label'),
    'An account has no paper key',
  );
  const settingsDot = settingsTab.querySelector('.rail-tail.dot');
  assert.ok(settingsDot, 'Settings carries its dot');
  assert.ok(settingsDot.classList.contains('warn'));
  assert.equal(
    settingsDot.getAttribute('aria-label'),
    'foks.partner.dev: not verified',
  );

  // Collapsed, every indicator still draws, in the same slot the stylesheet
  // folds to the icon's corner.
  await rail(
    { kind: 'all' },
    '0',
    0,
    true,
    false,
    { label: '2', description: '2 requests to join a team' },
    { description: 'An account has no paper key' },
    { description: 'foks.partner.dev: not verified' },
  );
  assert.ok(document.querySelector('.side.rail.is-narrow .rail-tail.count'));
  assert.equal(
    document.querySelectorAll('.side.rail.is-narrow .rail-tail.dot').length,
    2,
  );
});

test('with nothing known, Teams, Devices and Settings draw nothing', async () => {
  await rail({ kind: 'all' }, '0', 0);
  const [, , teamsTab, devicesTab, , settingsTab] = tabs();
  assert.equal(teamsTab.querySelector('.rail-tail'), null);
  assert.equal(devicesTab.querySelector('.rail-tail'), null);
  assert.equal(settingsTab.querySelector('.rail-tail'), null);
});

for (const agent of ['ready', 'starting', 'stopped', 'locked'] as const) {
  test(`the ${agent} rail has no connection footer in either width`, async () => {
    for (const collapsed of [false, true]) {
      const { rendered } = await rail(
        { kind: 'all' },
        '0',
        0,
        collapsed,
        false,
        null,
        null,
        null,
        agent,
      );
      assert.equal(
        Boolean(document.querySelector('.side.rail .status')),
        false,
      );
      assert.equal(Boolean(document.querySelector('.side.rail .foot')), false);
      rendered.unmount();
    }
  });
}

test('both setup rails omit the connection footer while keeping setup actions', async () => {
  const { SetupSidebar } = (await vite.ssrLoadModule(
    '/src/screens/first-run-view.tsx',
  )) as typeof import('../src/screens/first-run-view');
  const { initialFirstRun } = (await vite.ssrLoadModule(
    '/src/first-run-state.ts',
  )) as typeof import('../src/first-run-state');
  for (const managedLocal of [false, true]) {
    const view = ui.render(
      createElement(SetupSidebar, {
        checkpoint: { ...initialFirstRun('own', 'who'), managedLocal },
        agent: 'starting',
        onCancel: () => {},
      }),
    );
    assert.ok(view.getByRole('button', { name: 'Leave setup' }));
    assert.equal(Boolean(document.querySelector('.setup-side .status')), false);
    assert.equal(Boolean(document.querySelector('.setup-side .foot')), false);
    view.unmount();
  }
});

test('a tab click and Control-Tab both navigate over the six tabs', async () => {
  const { journal } = await rail({ kind: 'files' });
  ui.fireEvent.click(tabs()[4]);
  assert.deepEqual(journal.navigations.at(-1), { kind: 'people' });

  // Files is the first tab, so forward is Chat and backward wraps to Settings.
  ui.fireEvent.keyDown(document, { key: 'Tab', ctrlKey: true });
  assert.deepEqual(journal.navigations.at(-1), { kind: 'chat' });
  ui.fireEvent.keyDown(document, {
    key: 'Tab',
    ctrlKey: true,
    shiftKey: true,
  });
  assert.deepEqual(journal.navigations.at(-1), { kind: 'settings' });
  // A plain Tab is the browser's own; the rail ignores it.
  const before = journal.navigations.length;
  ui.fireEvent.keyDown(document, { key: 'Tab' });
  assert.equal(journal.navigations.length, before);
});

test('Control-Tab is disabled while a modal dialog is open', async () => {
  const { journal } = await rail({ kind: 'files' }, '0', 0, false, true);
  assert.ok(document.querySelector('[role="dialog"]'), 'the sheet is up');
  ui.fireEvent.keyDown(document, { key: 'Tab', ctrlKey: true });
  ui.fireEvent.keyDown(document, {
    key: 'Tab',
    ctrlKey: true,
    shiftKey: true,
  });
  // Modal state disables document-level tab shortcuts that the dialog cannot
  // intercept before they run.
  assert.deepEqual(journal.navigations, []);
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
    ['Files', 'Chat', 'Teams', 'Devices', 'Account', 'Settings'],
  );
});

test('the cycle helpers hold the rail order and wrap at both ends', async () => {
  const { sidebarCycleLocations, nextSidebarCycleLocation } =
    (await vite.ssrLoadModule(
      '/src/shell/sidebar.tsx',
    )) as typeof import('../src/shell/sidebar');
  assert.deepEqual(sidebarCycleLocations(), [
    { kind: 'files' },
    { kind: 'chat' },
    { kind: 'teams' },
    { kind: 'devices' },
    { kind: 'people' },
    { kind: 'settings' },
  ]);
  // Control-Tab walks the tab a location belongs to, not the location itself:
  // a group's settings page is on Teams, so forward from it is Devices.
  assert.deepEqual(
    nextSidebarCycleLocation({ kind: 'group-settings', ref: 'team:eng' }, 1),
    { kind: 'devices' },
  );
  assert.deepEqual(nextSidebarCycleLocation({ kind: 'store', ref: 'x' }, -1), {
    kind: 'settings',
  });
  // Both ends wrap.
  assert.deepEqual(nextSidebarCycleLocation({ kind: 'settings' }, 1), {
    kind: 'files',
  });
  assert.deepEqual(nextSidebarCycleLocation({ kind: 'files' }, -1), {
    kind: 'settings',
  });
  // First run belongs to no tab, so the walk starts at the end it came from.
  assert.deepEqual(
    nextSidebarCycleLocation({ kind: 'first-run', step: 'who' }, 1),
    { kind: 'files' },
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
  assert.equal(
    header.querySelector('.t small')?.textContent,
    'Personal server',
  );

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
  assert.ok(
    [...menu.querySelectorAll('.cap')].some(
      (entry) => entry.textContent === 'Personal server',
    ),
  );
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
    (button) => button.textContent === 'Add account or server…',
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

test('a stopped account is dimmed and carries the way to restore it', async () => {
  const { journal } = await rail({ kind: 'people', store: 'acct:personal' });
  const header = document.querySelector<HTMLButtonElement>('.side.rail .who');
  assert.ok(header);
  ui.fireEvent.click(header);
  const menu = await ui.waitFor(() => {
    const node = document.querySelector<HTMLElement>('[role="menu"]');
    assert.ok(node);
    return node;
  });
  // The current account is the one with the mark at the right end.
  const current = [...menu.querySelectorAll('.acct')].find((row) =>
    row.textContent?.includes('satoshi'),
  );
  assert.ok(current);
  assert.ok(current.classList.contains('on'));
  assert.equal(
    current.querySelector('.tick')?.getAttribute('aria-label'),
    'Current account',
  );
  const others = [...menu.querySelectorAll('.acct')].filter(
    (row) => row !== current,
  );
  assert.ok(others.every((row) => row.querySelector('.tick.off')));
  // Nothing in the fixture is stopped, so no account carries the amber line.
  assert.equal(menu.querySelector('.warnline'), null);
  assert.equal(journal.navigations.length, 0);
});

test('the rail names the account through which the selected group is accessed', async () => {
  await rail({ kind: 'group-settings', ref: 'team:eng' });
  assert.match(
    document.querySelector('.rail-account')?.textContent ??
      document.querySelector('.who')?.textContent ??
      '',
    /vitalik/,
  );
});

test('native titlebar controls follow rail collapse and expansion', async () => {
  const { App } = (await vite.ssrLoadModule(
    '/src/app-root.tsx',
  )) as typeof import('../src/app-root');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const calls: boolean[] = [];
  window.localStorage.clear();
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    setTrafficLightsVisible: async (visible) => {
      calls.push(visible);
    },
  };
  const r = ui.render(createElement(App, { snapshot: FIXTURE, bridge }));
  await ui.waitFor(() => assert.equal(calls.at(-1), true));
  ui.fireEvent.click(r.getByRole('button', { name: 'Collapse sidebar' }));
  await ui.waitFor(() => assert.equal(calls.at(-1), false));
  assert.ok(document.querySelector('.app.side-narrow'));
  ui.fireEvent.click(r.getByRole('button', { name: 'Expand sidebar' }));
  await ui.waitFor(() => assert.equal(calls.at(-1), true));
  window.localStorage.clear();
});

test('device crumbs use the name resolved for the exact account and address', async () => {
  const { crumbTrail } = (await vite.ssrLoadModule(
    '/src/shell/topbar.tsx',
  )) as typeof import('../src/shell/topbar');
  const location = {
    kind: 'devices',
    store: 'acct:work',
    device: '08abcd',
  } as const;
  const label = { store: 'acct:work', address: '08abcd', name: 'Travel key' };
  assert.deepEqual(crumbTrail(location, undefined, undefined, label), [
    'Devices',
    'Travel key',
  ]);
  assert.deepEqual(
    crumbTrail(location, undefined, undefined, {
      ...label,
      store: 'acct:personal',
    }),
    ['Devices', 'Key'],
  );
  assert.deepEqual(
    crumbTrail(location, undefined, undefined, { ...label, address: 'other' }),
    ['Devices', 'Key'],
  );
});

test('the settings crumb always names the sub-navigation’s open page', async () => {
  const { crumbTrail } = (await vite.ssrLoadModule(
    '/src/shell/topbar.tsx',
  )) as typeof import('../src/shell/topbar');
  // An address with no `section=` still opens a page — Servers, the
  // sub-navigation's first — so the crumb names it rather than stopping at
  // the tab.
  assert.deepEqual(crumbTrail({ kind: 'settings' }), ['Settings', 'Servers']);
  assert.deepEqual(crumbTrail({ kind: 'settings', section: 'mac' }), [
    'Settings',
    'Device',
  ]);
  // A server's own page reads three deep: the tab, the Servers page it
  // belongs to, and the server itself.
  assert.deepEqual(
    crumbTrail({ kind: 'settings', section: 'servers', profile: 'acme' }, {
      servers: [{ id: 'acme', name: 'internal-acme-profile', label: 'Acme' }],
    } as unknown as Parameters<typeof crumbTrail>[1]),
    ['Settings', 'Servers', 'Acme'],
  );
});
