/**
 * Tests for the Files list and the sidebar that drives it: the store tree,
 * the item kind rows, the list head, the rows' second lines, the view menu's
 * sorting and cards, and per-row action buttons across various store states.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import type { Location, LocationState, LocationStore } from '../src/location';
import { installDom } from './lib/dom-harness';

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
test.afterEach(() => {
  ui.cleanup();
  window.localStorage.removeItem('filesView');
  window.localStorage.removeItem('filesSidebarWidth');
});
test.after(async () => vite.close());

async function mount(
  location: Location,
  browsing?: Partial<LocationState>,
  /** What the page must have drawn before the test reads it. */
  ready = '.side-tree',
): Promise<{ store: LocationStore }> {
  const { App } = (await vite.ssrLoadModule(
    '/src/app-root.tsx',
  )) as typeof import('../src/app-root');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const { LocationStore, INITIAL_STATE } = (await vite.ssrLoadModule(
    '/src/location.ts',
  )) as typeof import('../src/location');
  const store = new LocationStore({ ...INITIAL_STATE, location, ...browsing });
  ui.render(
    createElement(App, {
      snapshot: FIXTURE,
      bridge: mockBridge(FIXTURE),
      store,
    }),
  );
  await ui.waitFor(() => assert.ok(document.querySelector(ready)));
  return { store };
}

/** Updates the browser search query managed by the window header. */
async function search(store: LocationStore, query: string): Promise<void> {
  await ui.act(async () => {
    store.search(query);
    await Promise.resolve();
  });
}

/** A tree row's own select button, found by the name it draws. */
function treeRow(name: string): HTMLButtonElement {
  const node = [
    ...document.querySelectorAll<HTMLButtonElement>('.side-tree .fselect'),
  ].find((button) => button.querySelector('.nm')?.textContent === name);
  assert.ok(node, `no tree row named ${name}`);
  return node;
}

/** Opens a tree row's disclosure, if it is closed. */
async function expand(name: string): Promise<void> {
  const twist =
    treeRow(name).parentElement?.querySelector<HTMLButtonElement>(
      ':scope > .twist',
    );
  assert.ok(twist, `${name} has a disclosure`);
  // The treeitem carries the disclosure state; the triangle is the
  // pointer's control.
  if (treeRow(name).getAttribute('aria-expanded') === 'true') return;
  ui.fireEvent.click(twist);
  await ui.waitFor(() =>
    assert.equal(treeRow(name).getAttribute('aria-expanded'), 'true'),
  );
}

/** A sidebar place row (`data-place`). */
function place(name: string): HTMLButtonElement {
  const row = document.querySelector<HTMLButtonElement>(
    `.side .nav[data-place="${name}"]`,
  );
  assert.ok(row, `the sidebar has a ${name} row`);
  return row;
}

/** The list head's title: the store, folder or search it names. */
function currentTitle(): string | undefined {
  return (
    document.querySelector('.lpane .list-head h2')?.textContent?.trim() ??
    undefined
  );
}

/** The list head's second line: the count and where it lives. */
function currentSubtitle(): string | undefined {
  return document.querySelector('.lpane .list-head p')?.textContent?.trim();
}

/** The list's rows, by name. */
function listRow(name: string): HTMLElement {
  const node = [...document.querySelectorAll<HTMLElement>('.lpane .row')].find(
    (row) => row.querySelector('.name .nm')?.textContent === name,
  );
  assert.ok(node, `no row named ${name}`);
  return node;
}

/** Opens the list bar's view menu and chooses one of its items. */
async function choose(label: string): Promise<void> {
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'View options' }));
  const item = await ui.screen.findByRole('menuitemradio', { name: label });
  ui.fireEvent.click(item);
  await ui.waitFor(() => assert.equal(ui.screen.queryByRole('menu'), null));
}

/** Whether the view menu has `label` checked, read by opening it. */
async function checked(label: string): Promise<boolean> {
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'View options' }));
  const item = await ui.screen.findByRole('menuitemradio', { name: label });
  const on = item.getAttribute('aria-checked') === 'true';
  ui.fireEvent.keyDown(document, { key: 'Escape' });
  await ui.waitFor(() => assert.equal(ui.screen.queryByRole('menu'), null));
  return on;
}

test('defaults to All Items, listing every store with its place on the second line', async () => {
  await mount({ kind: 'all' });
  // All Items is named by the sidebar alone; the list draws no head.
  assert.equal(currentTitle(), undefined);
  assert.equal(place('files').getAttribute('aria-current'), 'page');
  assert.equal(place('files').querySelector('.n')?.textContent, '13');
  assert.equal(document.querySelector('.lpane .path'), null);
  assert.equal(document.querySelector('.loc h1'), null);
  const rows = document.querySelectorAll('.lpane .body .row');
  assert.ok(rows.length > 1, 'items from more than one store are listed');
  // Items span stores, so each row's second line names its store.
  const places = new Set(
    [...rows].map(
      (row) => row.querySelector('.name small')?.textContent?.split(' › ')[0],
    ),
  );
  assert.ok(places.size > 1);
  // Search and New belong to the list bar above.
  assert.ok(document.querySelector('.topbar .topsearch'));
  assert.ok(ui.screen.getByRole('button', { name: 'New' }));
});

test('the item rows filter All Items by kind', async () => {
  const { store } = await mount({ kind: 'all' });
  assert.deepEqual(
    ['files', 'passwords', 'documents'].map(
      (name) => place(name).querySelector('.t')?.textContent,
    ),
    ['All Items', 'Passwords', 'Documents'],
  );
  ui.fireEvent.click(place('passwords'));
  await ui.waitFor(() => assert.equal(store.getSnapshot().kind, 'Password'));
  assert.equal(place('passwords').getAttribute('aria-current'), 'page');
  assert.equal(place('files').getAttribute('aria-current'), null);
  const tiles = [...document.querySelectorAll('.lpane .row .tile')];
  assert.ok(tiles.length > 0);
  assert.ok(tiles.every((tile) => tile.classList.contains('pw')));
  // Choosing a store shows all of its kinds again.
  ui.fireEvent.click(treeRow('Personal'));
  await ui.waitFor(() => assert.equal(store.getSnapshot().kind, 'All'));
});

test('stores follow under their headings, marked and counted', async () => {
  await mount({ kind: 'all' });
  assert.deepEqual(
    [...document.querySelectorAll('.side-tree .nav-h')].map((h) =>
      h.firstChild?.textContent?.trim(),
    ),
    ['Vaults', 'Teams'],
  );
  // Only a store with folders under it draws a disclosure, and trees start
  // collapsed.
  assert.ok(
    treeRow('Personal').parentElement?.querySelector(':scope > .twist'),
  );
  assert.equal(
    treeRow('Work (Acme)').parentElement?.querySelector(':scope > .twist'),
    null,
  );
  assert.equal(
    [...document.querySelectorAll('.side-tree .fselect .nm')].some(
      (node) => node.textContent === 'logins',
    ),
    false,
  );
  // Teams and personal vaults are marked with their initials.
  assert.equal(
    treeRow('Household').querySelector('.store-mark')?.textContent,
    'H',
  );
  assert.equal(
    treeRow('Personal').querySelector('.store-mark')?.textContent,
    'S',
  );
  // Item counts are shown for stores and, once open, their folders.
  assert.ok(treeRow('Personal').querySelector('.c'));
  await expand('Personal');
  assert.ok(treeRow('logins').querySelector('.c'));
  await expand('env');
  // Nested folders indent with their depth.
  assert.equal(
    treeRow('Personal').parentElement?.style.getPropertyValue('--d'),
    '0',
  );
  assert.equal(
    treeRow('logins').parentElement?.style.getPropertyValue('--d'),
    '1',
  );
  assert.equal(
    treeRow('prod').parentElement?.style.getPropertyValue('--d'),
    '2',
  );
});

test('a store with nothing in it is still reachable from the tree', async () => {
  await mount({ kind: 'all' });
  // "Work (Acme)" carries no items in the fixture.
  ui.fireEvent.click(treeRow('Work (Acme)'));
  await ui.waitFor(() => assert.equal(currentTitle(), 'Work (Acme)'));
  assert.equal(
    document.querySelector('.lpane .empty h2')?.textContent,
    'No Items',
  );
  assert.equal(
    document.querySelector('.lpane .empty p')?.textContent,
    'Passwords and documents you add are private to this vault.',
  );
  // The tree itself survives showing an empty store: Personal is still there.
  assert.ok(treeRow('Personal'));
});

test('an empty team displays shared-item details and its Manage command', async () => {
  const { store } = await mount({ kind: 'all' });
  // "Homelab" is a team with nothing in it whose setup did not finish: the
  // list says so, as its store page does, instead of an empty list.
  ui.fireEvent.click(treeRow('Homelab'));
  await ui.waitFor(() => assert.equal(currentTitle(), 'Homelab'));
  assert.equal(document.querySelector('.lpane .empty'), null);
  assert.equal(
    document.querySelector('.lpane .access-notice h2')?.textContent,
    'Setup incomplete',
  );
  assert.ok(ui.screen.getByRole('button', { name: 'Finish Setup' }));
  // A team's head offers its page.
  ui.fireEvent.click(treeRow('Household'));
  await ui.waitFor(() => assert.equal(currentTitle(), 'Household'));
  assert.match(currentSubtitle() ?? '', /3 items/);
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Manage…' }));
  await ui.waitFor(() =>
    assert.deepEqual(store.getSnapshot().location, {
      kind: 'group-settings',
      ref: 'team:household',
      tab: 'people',
    }),
  );
});

test('nested folders use explicit tree navigation', async () => {
  await mount({ kind: 'store', ref: 'acct:personal' }, { folder: '/env/prod' });
  assert.equal(currentTitle(), 'prod');
  // The head names the folders above the one that is open.
  assert.deepEqual(
    [...document.querySelectorAll('.lpane .list-head .crumb button')].map(
      (button) => button.textContent,
    ),
    ['Personal', 'env'],
  );
  // The selected folder's ancestors open in the tree so it stays visible.
  assert.equal(treeRow('prod').getAttribute('aria-current'), 'location');

  ui.fireEvent.click(treeRow('env'));
  await ui.waitFor(() => assert.equal(currentTitle(), 'env'));
  // The parent folder lists its child folder as a row with its item count.
  const folder = document.querySelector('.lpane .row.folder');
  assert.ok(folder);
  assert.equal(folder.querySelector('.name small')?.textContent, 'Folder');
  assert.equal(folder.querySelector('.r')?.textContent, '1 item');
});

test('a clicked row keeps the focus, and the arrow keys move the selection with it', async () => {
  const { store } = await mount({ kind: 'all' });
  const first = listRow('bundle.tar');
  ui.fireEvent.click(first);
  first.focus();
  await ui.waitFor(() =>
    assert.equal(store.getSnapshot().selection?.path, '/release/bundle.tar'),
  );
  assert.ok(first.classList.contains('sel'));
  ui.fireEvent.keyDown(first, { key: 'ArrowDown' });
  await ui.waitFor(() =>
    assert.equal(document.activeElement, listRow('DATABASE_URL')),
  );
  assert.equal(store.getSnapshot().selection?.path, '/env/prod/DATABASE_URL');
  ui.fireEvent.keyDown(document.activeElement!, { key: 'ArrowUp' });
  await ui.waitFor(() =>
    assert.equal(document.activeElement, listRow('bundle.tar')),
  );
});

test('the view menu is the sort control', async () => {
  const { store } = await mount({ kind: 'all' });
  assert.equal(await checked('Name'), true);
  await choose('Kind');
  await ui.waitFor(() => assert.equal(store.getSnapshot().sort, 'kind'));
  assert.equal(await checked('Kind'), true);
  assert.equal(await checked('Name'), false);
  // Kind order is the filter's own — Passwords, then Documents.
  const kinds = [...document.querySelectorAll('.lpane .row .tile')].map(
    (tile) => (tile.classList.contains('pw') ? 'Password' : 'Document'),
  );
  assert.ok(kinds.includes('Password') && kinds.includes('Document'));
  assert.ok(kinds.lastIndexOf('Password') < kinds.indexOf('Document'));
  await choose('Location');
  await ui.waitFor(() => assert.equal(store.getSnapshot().sort, 'group'));
});

test('a row names its item, its place, and the actions its kind allows', async () => {
  await mount({ kind: 'all' });
  const login = listRow('github.com');
  // The second line is the store and the folder.
  assert.equal(
    login.querySelector('.name small')?.textContent,
    'Personal › logins',
  );
  assert.ok(login.querySelector('.tile.pw'));
  // A value the panel reads as text copies and reveals; a file node
  // downloads. The row decides the same way the details panel does, so a
  // text Secret filed as a "Document" is not offered Download here and Copy
  // there.
  assert.deepEqual(actionsOf(login), ['Copy', 'Reveal']);
  // A Secret whose value carries no password line: kind "Document", read as
  // text all the same.
  const secretDocument = listRow('DATABASE_URL');
  assert.ok(secretDocument.querySelector('.tile.text'));
  assert.deepEqual(actionsOf(secretDocument), ['Copy', 'Reveal']);
  const fileDocument = listRow('passport-scan.pdf');
  assert.ok(fileDocument.querySelector('.tile.file'));
  assert.deepEqual(actionsOf(fileDocument), ['Download']);
});

test('cards preserve browsing state, sorting, and selected details and persist across remounts', async () => {
  const { store } = await mount(
    { kind: 'all' },
    {
      query: 'github',
      kind: 'Password',
      sort: 'name',
      sortDirection: 'desc',
      selection: { store: 'acct:personal', path: '/logins/github.com' },
    },
  );
  const before = store.getSnapshot();
  assert.equal(await checked('As List'), true);
  await choose('As Cards');
  assert.equal(store.getSnapshot(), before);
  assert.equal(await checked('As Cards'), true);
  assert.equal(document.querySelectorAll('.files-grid .row').length, 1);
  assert.equal(
    document.querySelector('.files-grid .row')?.getAttribute('aria-selected'),
    'true',
  );
  assert.equal(await checked('Descending'), true);
  await choose('Kind');
  await choose('Ascending');
  assert.equal(store.getSnapshot().sort, 'kind');
  assert.equal(store.getSnapshot().sortDirection, 'asc');
  await choose('As List');
  assert.equal(document.querySelector('.files-grid'), null);
  assert.equal(store.getSnapshot().query, 'github');
  assert.deepEqual(store.getSnapshot().selection, before.selection);
  assert.equal(store.getSnapshot().sort, 'kind');
  await choose('As Cards');
  ui.cleanup();
  await mount({ kind: 'all' });
  assert.ok(document.querySelector('.files-grid'));
  assert.equal(await checked('As Cards'), true);
});

test('folder cards navigate with the keyboard and retain the cards and kind filter', async () => {
  const { store } = await mount(
    { kind: 'store', ref: 'acct:personal' },
    { kind: 'Password' },
  );
  await choose('As Cards');
  const folder = document.querySelector<HTMLElement>(
    '.files-grid [data-folder-path="/logins"]',
  );
  assert.ok(folder);
  // At a store's root a first-level folder is the label of its section.
  assert.equal(folder.getAttribute('role'), 'option');
  assert.match(folder.getAttribute('aria-label') ?? '', /^logins, folder/);
  ui.fireEvent.keyDown(folder, { key: 'Enter' });
  assert.equal(store.getSnapshot().folder, '/logins');
  assert.equal(store.getSnapshot().kind, 'Password');
  assert.equal(currentTitle(), 'logins');
  assert.ok(document.querySelector('.files-grid .row:not(.folder)'));
});

test('search scopes to the selected tree row', async () => {
  const { store } = await mount({ kind: 'all' });
  ui.fireEvent.click(treeRow('Household'));
  await ui.waitFor(() => assert.equal(currentTitle(), 'Household'));
  // "github.com" lives in Personal, not Household.
  await search(store, 'github');
  await ui.waitFor(() =>
    assert.match(
      document.querySelector('.lpane .empty h2')?.textContent ?? '',
      /Nothing matches “github”/,
    ),
  );
  await search(store, 'netflix');
  await ui.waitFor(() =>
    assert.equal(document.querySelectorAll('.lpane .body .row').length, 1),
  );
  assert.equal(
    document.querySelector('.lpane .list-head .search-scope')?.textContent,
    'in Household',
  );
});

/** Returns a summary of each tree row containing its name, count, and selected state. */
function treeShape(): string[] {
  return [
    ...document.querySelectorAll<HTMLButtonElement>('.side-tree .fselect'),
  ].map((button) =>
    [
      button.querySelector('.nm')?.textContent ?? '',
      button.querySelector('.c')?.textContent ?? '',
      button.getAttribute('aria-current') ?? '',
    ].join('|'),
  );
}

test('typing a query leaves the folder tree unchanged', async () => {
  const { store } = await mount({ kind: 'all' });
  ui.fireEvent.click(treeRow('Personal'));
  await ui.waitFor(() => assert.equal(currentTitle(), 'Personal'));
  await expand('Personal');
  const before = treeShape();
  assert.ok(before.length > 1, 'the tree draws more than one row');
  // The tree is the browser's map: a search narrows the list beside it, not
  // the folders it is searching within, and does not change its aggregate.
  await search(store, 'github');
  await ui.waitFor(() =>
    assert.equal(document.querySelectorAll('.lpane .body .row').length, 1),
  );
  assert.deepEqual(treeShape(), before);
  // A query nothing matches empties the list and still leaves the tree whole.
  await search(store, 'no-such-item-anywhere');
  await ui.waitFor(() => assert.ok(document.querySelector('.lpane .empty')));
  assert.deepEqual(treeShape(), before);
  await search(store, '');
  await ui.waitFor(() =>
    assert.equal(document.querySelector('.lpane .empty'), null),
  );
  assert.deepEqual(treeShape(), before);
});

test('a missing store renders the unavailable state', async () => {
  await mount({ kind: 'store', ref: 'acct:gone' }, undefined, '.main .empty');
  assert.match(
    document.querySelector('.main .empty')?.textContent ?? '',
    /Vault Unavailable.*no longer on this device/,
  );
  // Neither the empty-folder invitation nor an empty list.
  assert.equal(document.querySelector('.list-window'), null);
  assert.equal(document.querySelector('.lpane .empty'), null);
});

for (const browsing of [{}, { query: 'logins' }]) {
  test(`Files reverses every sort key${'query' in browsing ? ' during search' : ''}`, async () => {
    const { store } = await mount({ kind: 'all' }, browsing);
    const rows = () =>
      [...document.querySelectorAll('.lpane .row .name .nm')].map(
        (node) => node.textContent,
      );
    for (const label of ['Name', 'Kind', 'Location']) {
      await choose(label);
      await choose('Ascending');
      await ui.waitFor(() =>
        assert.equal(store.getSnapshot().sortDirection, 'asc'),
      );
      const ascending = rows();
      assert.ok(ascending.length > 1);
      await choose('Descending');
      await ui.waitFor(() =>
        assert.deepEqual(rows(), [...ascending].reverse()),
      );
      assert.equal(store.getSnapshot().sortDirection, 'desc');
      await choose('Ascending');
      await ui.waitFor(() => assert.deepEqual(rows(), ascending));
    }
  });
}

test('folder sorting reverses folders and items while keeping folders first', async () => {
  const { store } = await mount(
    { kind: 'store', ref: 'acct:personal' },
    { sort: 'group' },
  );
  const rows = () =>
    [...document.querySelectorAll('.lpane .virtual-rows > *')].map(
      (node) => node.textContent,
    );
  const ascending = rows();
  assert.ok(ascending.length > 1);
  // Within one store, Location is not offered and the menu reads Name.
  assert.equal(await checked('Name'), true);
  await choose('Descending');
  await ui.waitFor(() =>
    assert.equal(store.getSnapshot().sortDirection, 'desc'),
  );
  assert.equal(store.getSnapshot().sort, 'name');
  assert.notDeepEqual(rows(), ascending);
  await choose('Ascending');
  await ui.waitFor(() => assert.deepEqual(rows(), ascending));
});

function actionsOf(row: Element): (string | null)[] {
  return [...row.querySelectorAll('.acts button')].map((button) =>
    button.getAttribute('title'),
  );
}

async function openTeamsPlus(): Promise<void> {
  const plus = document.querySelector<HTMLButtonElement>(
    '.side-tree .nav-h button[aria-label="Create or join a team"]',
  );
  assert.ok(plus, 'the Teams heading carries a create-or-join menu');
  assert.equal(plus.getAttribute('aria-haspopup'), 'menu');
  await ui.act(async () => {
    ui.fireEvent.click(plus);
    await Promise.resolve();
  });
}

test('the tree’s Teams menu offers Create, Join and Show All Teams', async () => {
  await mount({ kind: 'all' });
  await openTeamsPlus();
  const menu = await ui.screen.findByRole('menu', {
    name: 'Create or join a team',
  });
  assert.deepEqual(
    [...menu.querySelectorAll('button')].map((item) => item.textContent),
    ['Create a Team…', 'Join a Team…', 'Show All Teams'],
  );
});

for (const [item, heading] of [
  ['Create a Team…', 'Create a Team'],
  ['Join a Team…', 'Join a Team'],
] as const) {
  test(`the tree’s Teams menu: ${item} lands on Teams with its sheet up`, async () => {
    const { store } = await mount({ kind: 'all' });
    await openTeamsPlus();
    const menu = await ui.screen.findByRole('menu', {
      name: 'Create or join a team',
    });
    await ui.act(async () => {
      ui.fireEvent.click(ui.within(menu).getByText(item));
      await Promise.resolve();
    });
    // The sheet the item promises is open on the page it navigated to.
    await ui.waitFor(() =>
      assert.ok(
        [...document.querySelectorAll('.sheet h2')].some(
          (node) => node.textContent === heading,
        ),
        `the ${heading} sheet is open`,
      ),
    );
    // And the intent is spent: the address left behind is the plain list, so
    // the sheet is not reopened by a later render or by coming back.
    const { location } = store.getSnapshot();
    assert.equal(location.kind, 'teams');
    assert.equal(
      location.kind === 'teams' ? location.open : 'unset',
      undefined,
    );
  });
}

test('the tree’s Teams menu: Show All Teams opens Teams with no sheet', async () => {
  const { store } = await mount({ kind: 'all' });
  await openTeamsPlus();
  const menu = await ui.screen.findByRole('menu', {
    name: 'Create or join a team',
  });
  await ui.act(async () => {
    ui.fireEvent.click(ui.within(menu).getByText('Show All Teams'));
    await Promise.resolve();
  });
  await ui.waitFor(() =>
    assert.equal(store.getSnapshot().location.kind, 'teams'),
  );
  assert.equal(document.querySelector('.sheet'), null);
});
