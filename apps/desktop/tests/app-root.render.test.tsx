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
    assert.ok(document.querySelector('.side.rail .who .t b'));
  });
});

test.after(async () => {
  await vite.close();
});

test('the window is the rail and the content column, with no title bar', () => {
  assert.ok(
    document.querySelector('.window'),
    'window container element should exist',
  );
  assert.equal(document.querySelector('.titlebar'), null);
  // The window's drag strip is the top of the rail; the web mock draws the
  // three controls macOS would draw itself.
  assert.equal(
    document.querySelectorAll('.web-mock-window .side.rail .traffic .light')
      .length,
    3,
  );
  // Verify main navigation landmark exists.
  assert.ok(document.querySelector('nav.side[aria-label="Main Navigation"]'));
  assert.ok(document.querySelector('main.main > .topbar'));
  assert.equal(document.querySelector('.side .appname'), null);
});

test('the rail draws the six tabs, the unread badge and the attention dot', () => {
  const tabs = [
    ...document.querySelectorAll<HTMLButtonElement>(
      '.side.rail .rail-tabs .nav',
    ),
  ];
  assert.deepEqual(
    tabs.map((tab) => tab.querySelector('.t')?.textContent),
    ['Files', 'Chat', 'Teams', 'Devices', 'Accounts', 'Settings'],
  );
  // The fixture's two open notifications light the dot on the avatar, which is
  // the only place attention is advertised.
  const dot = document.querySelector('.side.rail .attn');
  assert.ok(dot, 'the account avatar carries the attention dot');
  assert.equal(dot.getAttribute('aria-label'), 'Needs attention');
  assert.equal(document.querySelector('.rail-tabs .dot'), null);
  // Files is the tab that owns All items, the shell's starting location.
  assert.equal(tabs[0].getAttribute('aria-current'), 'page');
  assert.equal(tabs[4].getAttribute('aria-current'), null);
});

test('the status light reports a connected service at the rail foot', () => {
  const light = document.querySelector('.side.rail .status');
  assert.ok(light);
  assert.ok(light.classList.contains('agent-ready'));
  assert.equal(light.textContent, 'Connected');
  assert.equal(light.getAttribute('title'), 'Connected');
});

test('the account header names the active account and opens its menu', async () => {
  const header = document.querySelector<HTMLButtonElement>('.side.rail .who');
  assert.ok(header, 'the rail draws the account header');
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
});

test('a tab navigates, and Control-Tab walks the six of them', async () => {
  const tab = (name: string): HTMLButtonElement => {
    const found = [
      ...document.querySelectorAll<HTMLButtonElement>('.side.rail .nav'),
    ].find((row) => row.querySelector('.t')?.textContent === name);
    assert.ok(found, `the rail has a ${name} tab`);
    return found;
  };
  testingLibrary.fireEvent.click(tab('Teams'));
  await testingLibrary.waitFor(() => {
    assert.equal(document.querySelector('.loc h1')?.textContent, 'Teams');
  });
  assert.equal(tab('Teams').getAttribute('aria-current'), 'page');

  // Control-Tab moves to the next tab in rail order; Shift walks back.
  testingLibrary.fireEvent.keyDown(document, { key: 'Tab', ctrlKey: true });
  await testingLibrary.waitFor(() => {
    assert.equal(tab('Devices').getAttribute('aria-current'), 'page');
  });
  testingLibrary.fireEvent.keyDown(document, {
    key: 'Tab',
    ctrlKey: true,
    shiftKey: true,
  });
  await testingLibrary.waitFor(() => {
    assert.equal(tab('Teams').getAttribute('aria-current'), 'page');
  });

  testingLibrary.fireEvent.click(tab('Files'));
  await testingLibrary.waitFor(() => {
    assert.equal(document.querySelector('.loc h1')?.textContent, 'All items');
  });
  testingLibrary.fireEvent.click(
    testingLibrary.screen.getByRole('button', { name: 'Back' }),
  );
});

test('the Files roots page lists the stores the rail used to enumerate', async () => {
  await testingLibrary.waitFor(() => {
    assert.equal(document.querySelector('.loc h1')?.textContent, 'Files');
  });
  const rows = [
    ...document.querySelectorAll<HTMLButtonElement>('.nav-rows .row'),
  ];
  const names = rows.map((row) => row.querySelector('.tt')?.textContent);
  assert.ok(names.includes('All items'));
  assert.ok(names.includes('Personal'));
  assert.ok(names.includes('Work (Acme)'));
  assert.ok(names.includes('Engineering'));

  const engineering = rows.find(
    (row) => row.querySelector('.tt')?.textContent === 'Engineering',
  );
  assert.ok(engineering);
  testingLibrary.fireEvent.click(engineering);
  await testingLibrary.waitFor(() => {
    assert.equal(document.querySelector('.loc h1')?.textContent, 'Engineering');
  });
  // The topbar names where the reader is, and the rail chevron returns to the tab's
  // root page.
  assert.equal(
    document.querySelector('.topbar .crumbs')?.textContent,
    'Files›Engineering',
  );
  const back = document.querySelector<HTMLButtonElement>(
    '.side.rail .rail-back',
  );
  assert.ok(back, 'the expanded rail carries the back chevron');
  assert.equal(back.disabled, false);
  testingLibrary.fireEvent.click(back);
  await testingLibrary.waitFor(() => {
    assert.equal(document.querySelector('.loc h1')?.textContent, 'Files');
  });
});

test('the chat tab opens a conversation and lists every team at once', async () => {
  const tab = [
    ...document.querySelectorAll<HTMLButtonElement>('.side.rail .nav'),
  ].find((row) => row.querySelector('.t')?.textContent === 'Chat');
  assert.ok(tab);
  testingLibrary.fireEvent.click(tab);
  const column = await testingLibrary.waitFor(() => {
    const node = document.querySelector('.chat-inbox');
    assert.ok(node, 'the chat tab draws its team column');
    return node;
  });
  const rows = [
    ...column.querySelectorAll<HTMLElement>('.chat-conv, .chat-team-head'),
  ];
  const name = (row: HTMLElement) => row.querySelector('b')?.textContent;
  const household = rows.find((row) => name(row) === 'Household');
  const engineering = rows.find((row) => name(row) === 'Engineering');
  assert.ok(household, 'a team whose server offers chat is listed');
  assert.ok(engineering, 'a team whose server offers no chat is still listed');
  // `chat` with no conversation resolves to one, and its row is the current
  // one. Household's channels are the general channel alone, so it is a single
  // row rather than a heading with a list.
  await testingLibrary.waitFor(() =>
    assert.equal(household.getAttribute('aria-current'), 'page'),
  );
  // Chat follows the server capability grant, as the rail's chat rows did:
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

test('opens on All items, in the folder browser', async () => {
  // This runs after the walk above, so return to the starting location.
  const files = [
    ...document.querySelectorAll<HTMLButtonElement>('.side.rail .nav'),
  ].find((row) => row.querySelector('.t')?.textContent === 'Files');
  assert.ok(files);
  testingLibrary.fireEvent.click(files);
  await testingLibrary.waitFor(() => {
    assert.ok(document.querySelector('.nav-rows .row'));
  });
  const all = [
    ...document.querySelectorAll<HTMLButtonElement>('.nav-rows .row'),
  ].find((row) => row.querySelector('.tt')?.textContent === 'All items');
  assert.ok(all);
  testingLibrary.fireEvent.click(all);
  await testingLibrary.waitFor(() => {
    assert.equal(document.querySelector('.loc h1')?.textContent, 'All items');
  });
  assert.ok(
    document.querySelector('.folder-layout'),
    'item pages open in the folder browser',
  );
  const rows = document.querySelectorAll('.body .row');
  assert.ok(rows.length > 0, 'items list is rendered in main body');
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

  const tab = (name: string): HTMLButtonElement => {
    const found = [
      ...document.querySelectorAll<HTMLButtonElement>('.side.rail .nav'),
    ].find((row) => row.querySelector('.t')?.textContent === name);
    assert.ok(found, `the rail has a ${name} tab`);
    return found;
  };
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

  for (const [name, settled] of [
    ['Accounts', 'Actions on this account'],
    ['Devices', 'Computers and security keys'],
    ['Settings', 'Danger zone'],
  ] as const) {
    testingLibrary.fireEvent.click(tab(name));
    const main = await testingLibrary.waitFor(() => {
      assert.equal(document.querySelector('.loc h1')?.textContent, name);
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
