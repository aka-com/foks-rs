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
  await testingLibrary.waitFor(() => {
    assert.ok(document.querySelector('.app'));
    assert.equal(document.querySelector('.app-loading'), null);
  });
});

test.after(async () => {
  await vite.close();
});

test('renders shell frame with application brand name', () => {
  assert.ok(
    document.querySelector('.window'),
    'window container element should exist',
  );
  assert.equal(document.querySelector('.titlebar .brand')?.textContent, 'FOKS');
  assert.equal(
    document.querySelectorAll('.web-mock-window .lights .light').length,
    3,
  );
  // Verify main navigation landmark exists.
  assert.ok(document.querySelector('nav.side[aria-label="Main Navigation"]'));
  assert.ok(document.querySelector('main.main'));
  // Brand name is rendered in the title bar rather than sidebar navigation.
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
    ['People', 'Chat', 'Files', 'Teams', 'Devices', 'Settings'],
  );
  // The fixture's two open notifications light the People dot.
  assert.ok(tabs[0].querySelector('.dot'), 'People carries the attention dot');
  // Files is the tab that owns All items, the shell's starting location.
  assert.equal(tabs[2].getAttribute('aria-current'), 'page');
  assert.equal(tabs[0].getAttribute('aria-current'), null);
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
  assert.ok(labels.some((text) => text === 'Add an account or server…'));
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
    assert.equal(document.querySelector('.loc h1')?.textContent, 'Files');
  });
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
  // The items page returns to the roots page it was opened from.
  const back = document.querySelector<HTMLButtonElement>('.path .page-back');
  assert.ok(back, 'the items header carries a back chevron');
  assert.equal(back.getAttribute('aria-label'), 'Back to Files');
  testingLibrary.fireEvent.click(back);
  await testingLibrary.waitFor(() => {
    assert.equal(document.querySelector('.loc h1')?.textContent, 'Files');
  });
});

test('the chat tab opens the first team with chat and lists the rest', async () => {
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
  const rows = [...column.querySelectorAll<HTMLElement>('.chat-team-head')];
  const name = (row: HTMLElement) => row.querySelector('b')?.textContent;
  const household = rows.find((row) => name(row) === 'Household');
  const engineering = rows.find((row) => name(row) === 'Engineering');
  assert.ok(household, 'a team whose server offers chat is a heading');
  assert.ok(engineering, 'a team whose server offers no chat is still listed');
  // `chat` with no team resolves to the first team that has one, and that team
  // is the open one.
  assert.equal(household.getAttribute('aria-current'), 'true');
  // Chat follows the server capability grant, as the rail's chat rows did:
  // Engineering's server offers none, so it sits under "No chat", dimmed and
  // not selectable.
  assert.ok(engineering.classList.contains('off'));
  assert.equal(engineering.getAttribute('role'), null);
  assert.ok(
    [...column.querySelectorAll('.sec')].some(
      (label) => label.textContent === 'No chat',
    ),
    'the No chat group names itself',
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

test.after(() => {
  dom.window.close();
});
