/**
 * Renders and verifies the primary application shell using jsdom and Vite SSR loader.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { ViteDevServer } from 'vite';

import { installDom } from './lib/dom-harness';

const dom = installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
});

type TestingLibrary = typeof import('@testing-library/react');
let testingLibrary: TestingLibrary;
let vite: ViteDevServer;

test.before(async () => {
  testingLibrary = await import('@testing-library/react');
  vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
  await vite.ssrLoadModule('/src/main.tsx');
  // The frame is drawn before the snapshot arrives, so the account header is
  // what says the shell has finished booting.
  await testingLibrary.waitFor(() => {
    assert.ok(document.querySelector('.app'));
    assert.ok(document.querySelector('.side .who .t b'));
  });
});

test.after(async () => {
  await vite.close();
});

test('the window is the sidebar and the raised panel, with no title bar', () => {
  assert.ok(
    document.querySelector('.window'),
    'window container element should exist',
  );
  assert.equal(document.querySelector('.titlebar'), null);
  // The window's drag strip is the top of the sidebar; the web mock draws
  // the three controls macOS would draw itself.
  assert.equal(
    document.querySelectorAll('.web-mock-window .side .traffic .light').length,
    3,
  );
  // Verify main navigation landmark exists.
  assert.ok(document.querySelector('nav.side[aria-label="Main Navigation"]'));
  assert.ok(document.querySelector('main.main > .topbar'));
  assert.equal(document.querySelector('.side .appname'), null);
});

/** A sidebar row by the place it stands for (`data-place`). */
function place(name: string): HTMLButtonElement {
  const row = document.querySelector<HTMLButtonElement>(
    `.side .nav[data-place="${name}"]`,
  );
  assert.ok(row, `the sidebar has a ${name} row`);
  return row;
}

test('the sidebar draws its places, the request count and the Settings dot', async () => {
  assert.deepEqual(
    [
      ...document.querySelectorAll<HTMLButtonElement>('.side .nav[data-place]'),
    ].map((row) => row.querySelector('.t')?.textContent),
    [
      'All Items',
      'Passwords',
      'Documents',
      'All Teams',
      'Chat',
      'Activity',
      'Devices',
      'Settings',
    ],
  );
  // The fixture's three notifications each already have a home of their own —
  // Acme's lapsed check-in and Partner's unverified trust show on Settings,
  // Homelab's incomplete setup and its inactive federated membership show on
  // its own row — so none of them are left for the avatar dot to advertise.
  assert.equal(
    document.querySelector('.side .attn'),
    null,
    'nothing is left unrouted for the avatar dot to carry',
  );
  // Partner has never been verified (Acme's own lapsed check-in note is only
  // live once its lease is actually expired, which the running demo agent
  // has not made true here), so the Settings row carries the amber dot for
  // Partner alone.
  const settingsDot = place('settings').querySelector('.nav-tail.dot');
  assert.ok(settingsDot, 'the Settings row carries its own dot');
  assert.ok(settingsDot.classList.contains('warn'));
  assert.equal(
    settingsDot.getAttribute('aria-label'),
    'foks.partner.dev: not verified',
  );
  // All Teams' count reads the shared request-count rows, loaded on unlock
  // for every named team this account can manage: the fixture's inbox holds
  // two requests for each of its two manageable teams. The Devices dot still
  // depends on the Devices page having loaded its signal this session, and
  // that page has not been visited, so it is silent rather than guessing.
  await testingLibrary.waitFor(() =>
    assert.equal(
      place('teams').querySelector('.nav-tail.count')?.textContent ?? null,
      '4',
    ),
  );
  assert.equal(place('devices').querySelector('.nav-tail'), null);
  // All Items is the row that owns the shell's starting location.
  assert.equal(place('files').getAttribute('aria-current'), 'page');
  assert.equal(place('settings').getAttribute('aria-current'), null);
});

test('the sidebar foot carries the sync status line under the account', () => {
  const foot = document.querySelector('.side .side-foot');
  assert.ok(foot);
  assert.ok(foot.querySelector('.side-status .global-refresh'));
  assert.equal(Boolean(document.querySelector('.side .foot')), false);
});

test('the account header names the active account and opens its menu', async () => {
  const header = document.querySelector<HTMLButtonElement>('.side .who');
  assert.ok(header, 'the sidebar draws the account header');
  assert.equal(header.querySelector('.t b')?.textContent, 'satoshi');
  assert.equal(header.getAttribute('aria-expanded'), 'false');

  testingLibrary.fireEvent.click(header);
  const menu = await testingLibrary.waitFor(() => {
    const node = document.querySelector('[role="menu"]');
    assert.ok(node, 'the account menu opens');
    return node;
  });
  assert.equal(header.getAttribute('aria-expanded'), 'true');
  const labels = [...menu.querySelectorAll('button')].map((button) =>
    button.textContent?.trim(),
  );
  assert.ok(labels.some((text) => text?.includes('satoshi')));
  assert.ok(labels.some((text) => text === 'Add account or server…'));
  assert.ok(labels.some((text) => text === 'Lock'));
  testingLibrary.fireEvent.keyDown(document, { key: 'Escape' });
});

test('a row navigates, and Control-Tab walks the places in sidebar order', async () => {
  testingLibrary.fireEvent.click(place('teams'));
  // The page is named once, by the bar's crumb.
  await testingLibrary.waitFor(() => {
    assert.equal(
      document.querySelector('.topbar .crumbs [aria-current="page"]')
        ?.textContent,
      'Teams',
    );
  });
  assert.equal(place('teams').getAttribute('aria-current'), 'page');

  // Control-Tab moves to the next place in sidebar order; Shift walks back.
  testingLibrary.fireEvent.keyDown(document, { key: 'Tab', ctrlKey: true });
  await testingLibrary.waitFor(() => {
    assert.equal(place('chat').getAttribute('aria-current'), 'page');
  });
  testingLibrary.fireEvent.keyDown(document, {
    key: 'Tab',
    ctrlKey: true,
    shiftKey: true,
  });
  await testingLibrary.waitFor(() => {
    assert.equal(place('teams').getAttribute('aria-current'), 'page');
  });

  testingLibrary.fireEvent.click(place('files'));
  await testingLibrary.waitFor(() => {
    assert.equal(place('files').getAttribute('aria-current'), 'page');
  });
  assert.ok(document.querySelector('.lpane'));
});

/** The sidebar's store tree rows, by the name each draws. */
function treeRows(): HTMLButtonElement[] {
  return [
    ...document.querySelectorAll<HTMLButtonElement>('.side-tree .fselect'),
  ];
}

test('the sidebar tree lists the stores and opens one in the list', async () => {
  const names = treeRows().map((row) => row.querySelector('.nm')?.textContent);
  assert.ok(names.includes('Personal'));
  assert.ok(names.includes('Work (Acme)'));
  assert.ok(names.includes('Engineering'));

  const engineering = treeRows().find(
    (row) => row.querySelector('.nm')?.textContent === 'Engineering',
  );
  assert.ok(engineering);
  testingLibrary.fireEvent.click(engineering);
  // Browsing a store through the tree is client-side selection, not a
  // location change; the list's head names the store and the row is current.
  await testingLibrary.waitFor(() => {
    assert.equal(
      document.querySelector('.lpane .list-head h2')?.textContent,
      'Engineering',
    );
  });
  assert.equal(engineering.getAttribute('aria-current'), 'location');
  assert.equal(place('files').getAttribute('aria-current'), null);
  testingLibrary.fireEvent.click(place('files'));
  await testingLibrary.waitFor(() => {
    assert.equal(place('files').getAttribute('aria-current'), 'page');
  });
  assert.equal(document.querySelector('.lpane .list-head'), null);
});

test('the chat tab opens a conversation and lists every team at once', async () => {
  testingLibrary.fireEvent.click(place('chat'));
  const column = await testingLibrary.waitFor(() => {
    const node = document.querySelector('.chat-inbox');
    assert.ok(node, 'the chat tab draws its team column');
    return node;
  });
  const rows = [...column.querySelectorAll<HTMLElement>('.chat-team-head')];
  const name = (row: HTMLElement) => row.querySelector('b')?.textContent;
  const household = rows.find((row) => name(row) === 'Household');
  const engineering = rows.find((row) => name(row) === 'Engineering');
  assert.ok(household, 'a team whose server offers chat is listed');
  assert.ok(engineering, 'a team whose server offers no chat is still listed');
  // `chat` with no conversation resolves to one, and that channel's row under
  // Household's heading is the current one.
  await testingLibrary.waitFor(() =>
    assert.equal(
      household
        .closest('.chat-team')
        ?.querySelector('.chat-channel')
        ?.getAttribute('aria-current'),
      'page',
    ),
  );
  // Chat follows the server capability grant, as the old rail's chat rows did:
  // Engineering's server offers none, so it sits under "Chat unavailable", dimmed and
  // not selectable.
  assert.ok(engineering.classList.contains('off'));
  assert.equal(engineering.getAttribute('role'), null);
  assert.ok(
    [...column.querySelectorAll('.sec')].some(
      (label) => label.textContent === 'Chat unavailable',
    ),
    'the Chat unavailable section header is displayed',
  );
  // The location remembers the team the tab chose.
  await testingLibrary.waitFor(() => {
    assert.match(window.location.search, /state=chat&?/);
    assert.match(window.location.search, /store=team%3Ahousehold/);
  });
});

test('renders navigation icons as SVG elements', () => {
  const icon = document.querySelector('.side .nav .ic');
  assert.ok(icon, 'a nav row has an icon');
  assert.equal(icon.getAttribute('viewBox'), '0 0 24 24');
  assert.equal(icon.getAttribute('stroke'), 'currentColor');
  assert.equal(icon.getAttribute('fill'), 'none');
  assert.equal(icon.getAttribute('stroke-width'), '1.7');
  assert.ok(icon.children.length > 0, 'icon contains SVG child elements');
});

test('opens on All Items, in the list column', async () => {
  // This runs after the walk above, so return to the starting location.
  testingLibrary.fireEvent.click(place('files'));
  await testingLibrary.waitFor(() => {
    assert.ok(document.querySelector('.lpane'));
  });
  assert.equal(place('files').getAttribute('aria-current'), 'page');
  const rows = document.querySelectorAll('.lpane .body .row');
  assert.ok(rows.length > 0, 'items list is rendered in the list column');
  // Nothing is selected, so the details column says so.
  assert.ok(document.querySelector('.details.details-empty'));
  assert.equal(document.querySelector('[data-shell-placeholder]'), null);
});

/**
 * A StoreRef is the agent's own identifier for a store and is opaque: the real
 * agent answers with a JSON object, not the fixture's readable `acct:<alias>`.
 * The address may carry one — `?store=` has always encoded it — but no page may
 * draw one, so Accounts, Devices and Settings are read for every ref the catalog
 * holds, in their text and in the attributes a reader is shown.
 */
test('Accounts, Devices and Settings draw no StoreRef', async () => {
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const refs = FIXTURE.stores.map((store) => store.id);
  assert.ok(refs.includes('acct:personal'), 'the fixture names account stores');
  assert.ok(refs.includes('team:eng'), 'the fixture names team stores');

  /** Everything the page shows a reader: its text and its shown attributes. */
  const shownText = (main: Element): string =>
    [
      main.textContent ?? '',
      ...[
        ...main.querySelectorAll('[title],[aria-label],[placeholder]'),
      ].flatMap((node) =>
        ['title', 'aria-label', 'placeholder'].map(
          (name) => node.getAttribute(name) ?? '',
        ),
      ),
      ...[...main.querySelectorAll('input,textarea')].map(
        (node) => (node as HTMLInputElement).value,
      ),
    ].join(' ');

  for (const [name, heading, settled] of [
    ['Devices', 'Devices', 'paper-backup'],
    // Settings' sub-navigation opens on Account, its landing page, whose
    // header names the account it is about rather than the section.
    ['Settings', 'satoshi', 'devices and keys'],
  ] as const) {
    testingLibrary.fireEvent.click(place(name.toLowerCase()));
    const main = await testingLibrary.waitFor(() => {
      assert.equal(document.querySelector('.loc h1')?.textContent, heading);
      const node = document.querySelector('main.main');
      assert.ok(node);
      // Wait for the page's own reads: a loading pane draws none of the rows
      // a ref could reach, so reading it early would pass for the wrong reason.
      assert.ok(
        (node.textContent ?? '').includes(settled),
        `${name} finished loading`,
      );
      return node;
    });
    const drawn = shownText(main);
    for (const ref of refs)
      assert.equal(
        drawn.includes(ref),
        false,
        `${name} draws the StoreRef ${ref}`,
      );
  }
});

test.after(() => {
  dom.window.close();
});
