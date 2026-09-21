/**
 * Tests for the Files browser view, covering the tree navigation column,
 * item kind filters, folder title headers, item table columns, sorting,
 * and per-row action buttons across various store states.
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
test.afterEach(() => ui.cleanup());
test.after(async () => vite.close());

async function mount(
  location: Location,
  browsing?: Partial<LocationState>,
  /** What the page must have drawn before the test reads it. */
  ready = '.tpane',
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
    ...document.querySelectorAll<HTMLButtonElement>('.tpane .fselect'),
  ].find((button) => button.querySelector('.nm')?.textContent === name);
  assert.ok(node, `no tree row named ${name}`);
  return node;
}

/** Returns the title of the currently open folder in the list column. */
function currentTitle(): string {
  const where = document.querySelector('.lpane .lt .where');
  assert.ok(where, 'expected list column title row to be present');
  const copy = where.cloneNode(true) as Element;
  for (const extra of copy.querySelectorAll('.sub, .kico, .av')) extra.remove();
  return copy.textContent?.trim() ?? '';
}

/** Returns the item count subtitle displayed beside the folder name. */
function currentSubtitle(): string | undefined {
  return document.querySelector('.lpane .lt .where .sub')?.textContent?.trim();
}

/** The table header's column labels, sort arrow stripped. */
function columns(): string[] {
  return [...document.querySelectorAll('.lpane .hdr > *')]
    .map((cell) => cell.textContent?.replace(/ [↑↓]/, '').trim() ?? '')
    .filter(Boolean);
}

test('defaults to All items, displaying all stores in one table', async () => {
  await mount({ kind: 'all' });
  assert.equal(currentTitle(), 'All items');
  assert.equal(currentSubtitle(), '13 items');
  assert.equal(treeRow('All items').getAttribute('aria-current'), 'location');
  // The page does not render a separate header; the list column's title row serves as the header.
  assert.equal(document.querySelector('.lpane .path'), null);
  assert.equal(document.querySelector('.loc h1'), null);
  const rows = document.querySelectorAll('.lpane .body .row');
  assert.ok(rows.length > 1, 'items from more than one store are listed');
  // Items span stores, so Location identifies each item's store.
  assert.deepEqual(columns(), ['Name', 'Kind', 'Location', 'Size']);
  assert.equal(document.querySelector('.rail-back'), null);
  // The browser draws no toolbar band of its own any more: search and New
  // belong to the window header.
  assert.equal(document.querySelector('.folder-layout .toolbar'), null);
});

test('renders kind filter at the top of the tree column', async () => {
  await mount({ kind: 'all' });
  const filter = document.querySelector(
    '.tpane > .filt [aria-label="Filter items by kind"]',
  );
  assert.ok(filter, 'expected kind filter to be present');
  assert.deepEqual(
    [...filter.querySelectorAll('button')].map((button) =>
      button.textContent?.trim(),
    ),
    ['All', 'Passwords', 'Documents'],
  );
  const pane = filter.closest('.tpane');
  assert.ok(pane);
  assert.ok(pane.children[0]?.classList.contains('filt'));
  assert.ok(pane.children[1]?.classList.contains('tscroll'));
  assert.ok(
    pane.querySelector('.tscroll .fn'),
    'expected tree items to render in scrollable container below filter',
  );
});

test('All items is the first tree row; stores follow under their headings', async () => {
  await mount({ kind: 'all' });
  const names = [...document.querySelectorAll('.tpane .fselect .nm')].map(
    (node) => node.textContent,
  );
  assert.equal(names[0], 'All items');
  assert.deepEqual(
    [...document.querySelectorAll('.tpane h6')].map((h) =>
      h.firstChild?.textContent?.trim(),
    ),
    ['Vaults', 'Teams'],
  );
  // Every row keeps a twist cell so names line up down the column; only a
  // row with folders under it puts a button in that cell.
  assert.ok(document.querySelector('.tpane .fn.root .twist:not(.none)'));
  assert.ok(document.querySelector('.tpane .fn .twist.none'));
  // Teams and personal vaults are marked with their respective initials.
  assert.ok(treeRow('Household').querySelector('.av.team'));
  const accountMark = treeRow('Personal').querySelector('.kico.account');
  assert.ok(accountMark);
  assert.equal(accountMark.textContent, 'S');
  assert.equal(treeRow('Personal').querySelector('.fselect > .ic'), null);
  // Item counts are shown for all tree rows: the total aggregate, individual stores, and subfolders.
  assert.equal(
    treeRow('All items').parentElement?.querySelector(':scope > .c')
      ?.textContent,
    '13',
  );
  assert.ok(treeRow('Personal').parentElement?.querySelector(':scope > .c'));
  assert.ok(treeRow('logins').parentElement?.querySelector(':scope > .c'));
  // Nested folders indent incrementally based on their folder depth.
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
    'No items yet',
  );
  assert.equal(
    document.querySelector('.lpane .empty p')?.textContent,
    'Passwords and documents you add here are private to this vault.',
  );
  // The tree itself survives showing an empty store: Personal is still there.
  assert.ok(treeRow('Personal'));
});

test('an empty team displays shared-item details and inline settings', async () => {
  await mount({ kind: 'all' });
  // "Homelab" is a team with nothing in it.
  assert.ok(
    treeRow('Homelab').parentElement?.querySelector(
      '.fact[aria-label="Homelab settings"]',
    ),
    'team settings is mounted before selection so row hover can reveal it',
  );
  ui.fireEvent.click(treeRow('Homelab'));
  await ui.waitFor(() => assert.equal(currentTitle(), 'Homelab'));
  assert.equal(
    document.querySelector('.lpane .empty p')?.textContent,
    'Items stored in this team vault are accessible to team members according to their assigned roles.',
  );
  assert.ok(
    treeRow('Homelab').parentElement?.querySelector(
      '.fact[aria-label="Homelab settings"]',
    ),
  );
});

test('nested folders use explicit tree navigation', async () => {
  await mount({ kind: 'store', ref: 'acct:personal' }, { folder: '/env/prod' });
  assert.equal(currentTitle(), 'prod');
  assert.deepEqual(columns(), ['Name', 'Kind', 'Size']);

  ui.fireEvent.click(treeRow('env'));
  await ui.waitFor(() => assert.equal(currentTitle(), 'env'));
  // The parent folder lists its child folder as a row with its item count.
  const folder = document.querySelector('.lpane .row.folder');
  assert.ok(folder);
  assert.deepEqual(
    [...folder.querySelectorAll('.cell')].map((cell) => cell.textContent),
    ['Folder', '1 item'],
  );
});

test('the column headers are the sort control', async () => {
  await mount({ kind: 'all' });
  const header = (label: string): HTMLButtonElement => {
    const node = [
      ...document.querySelectorAll<HTMLButtonElement>('.lpane .hdr button'),
    ].find((button) => button.textContent?.startsWith(label));
    assert.ok(node, `no column named ${label}`);
    return node;
  };
  assert.equal(header('Name').getAttribute('aria-pressed'), 'true');
  ui.fireEvent.click(header('Kind'));
  await ui.waitFor(() =>
    assert.equal(header('Kind').getAttribute('aria-pressed'), 'true'),
  );
  assert.equal(header('Name').getAttribute('aria-pressed'), 'false');
  // Kind order is the filter's own — Passwords, then Documents.
  const kinds = [...document.querySelectorAll('.lpane .row .cell.kind')].map(
    (cell) => cell.textContent,
  );
  assert.ok(kinds.includes('Password') && kinds.includes('Document'));
  assert.ok(kinds.lastIndexOf('Password') < kinds.indexOf('Document'));
  ui.fireEvent.click(header('Location'));
  await ui.waitFor(() =>
    assert.equal(header('Location').getAttribute('aria-pressed'), 'true'),
  );
});

test('a row names its item, its folder, and the actions its kind allows', async () => {
  await mount({ kind: 'all' });
  const rows = [...document.querySelectorAll('.lpane .row')];
  const login = rows.find(
    (candidate) =>
      candidate.querySelector('.name .nm')?.textContent === 'github.com',
  );
  assert.ok(login, 'the GitHub login is listed');
  // The folder follows the name, in place of the chip the row used to carry.
  assert.equal(login.querySelector('.name .fpath')?.textContent, '/logins');
  assert.equal(login.querySelector('.pchip'), null);
  // A value the panel reads as text copies and reveals; a file node
  // downloads. The row decides the same way the details panel does, so a
  // text Secret filed as a "Document" is not offered Download here and Copy
  // there.
  assert.deepEqual(actionsOf(login), ['Copy', 'Reveal']);
  const named = (name: string): Element => {
    const node = rows.find(
      (candidate) => candidate.querySelector('.name .nm')?.textContent === name,
    );
    assert.ok(node, `no row named ${name}`);
    return node;
  };
  // A Secret whose value carries no password line: kind "Document", read as
  // text all the same.
  const secretDocument = named('DATABASE_URL');
  assert.equal(
    secretDocument.querySelector('.cell.kind')?.textContent,
    'Document',
  );
  assert.deepEqual(actionsOf(secretDocument), ['Copy', 'Reveal']);
  const fileDocument = named('passport-scan.pdf');
  assert.equal(
    fileDocument.querySelector('.cell.kind')?.textContent,
    'Document',
  );
  assert.deepEqual(actionsOf(fileDocument), ['Download']);
});

test('disables grid view button when grid view is not yet supported', async () => {
  await mount({ kind: 'all' });
  const view = document.querySelector('.lpane .lt [aria-label="View"]');
  assert.ok(view);
  const [list, grid] = [...view.querySelectorAll('button')];
  assert.equal(list.className, 'on');
  assert.equal(grid.getAttribute('aria-disabled'), 'true');
  assert.equal(grid.getAttribute('title'), 'Grid view is not available yet');
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
});

/** Returns a summary of each tree row containing its name, count, and selected state. */
function treeShape(): string[] {
  return [
    ...document.querySelectorAll<HTMLButtonElement>('.tpane .fselect'),
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
  await mount({ kind: 'store', ref: 'acct:gone' }, undefined, '.notice');
  assert.match(
    document.querySelector('.notice')?.textContent ?? '',
    /Vault no longer available/,
  );
  // Neither the empty-folder invitation nor a bare table of column headings.
  assert.equal(document.querySelector('.list-window'), null);
  assert.equal(document.querySelector('.empty'), null);
});

for (const browsing of [{}, { query: 'logins' }]) {
  test(`Files reverses every sortable column${'query' in browsing ? ' during search' : ''}`, async () => {
    const { store } = await mount({ kind: 'all' }, browsing);
    const rows = () =>
      [...document.querySelectorAll('.lpane .row .name .nm')].map(
        (node) => node.textContent,
      );
    for (const label of ['Name', 'Kind', 'Location']) {
      const header = () =>
        [
          ...document.querySelectorAll<HTMLButtonElement>('.lpane .hdr button'),
        ].find((node) => node.textContent?.startsWith(label))!;
      if (header().getAttribute('aria-pressed') !== 'true')
        ui.fireEvent.click(header());
      await ui.waitFor(() =>
        assert.equal(store.getSnapshot().sortDirection, 'asc'),
      );
      const ascending = rows();
      assert.ok(ascending.length > 1);
      ui.fireEvent.click(header());
      await ui.waitFor(() =>
        assert.deepEqual(rows(), [...ascending].reverse()),
      );
      assert.equal(store.getSnapshot().sortDirection, 'desc');
      assert.match(header().textContent ?? '', /↓/);
      ui.fireEvent.click(header());
      await ui.waitFor(() => assert.deepEqual(rows(), ascending));
      assert.match(header().textContent ?? '', /↑/);
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
  const name = [
    ...document.querySelectorAll<HTMLButtonElement>('.lpane .hdr button'),
  ].find((node) => node.textContent?.startsWith('Name'))!;
  ui.fireEvent.click(name);
  await ui.waitFor(() =>
    assert.equal(store.getSnapshot().sortDirection, 'desc'),
  );
  assert.equal(store.getSnapshot().sort, 'name');
  assert.notDeepEqual(rows(), ascending);
  ui.fireEvent.click(name);
  await ui.waitFor(() => assert.deepEqual(rows(), ascending));
});

function actionsOf(row: Element): (string | null)[] {
  return [...row.querySelectorAll('.acts button')].map((button) =>
    button.getAttribute('title'),
  );
}

test('the tree’s New team button lands on Teams with the create sheet up', async () => {
  const { store } = await mount({ kind: 'all' });
  const plus = document.querySelector<HTMLButtonElement>(
    '.tpane button.plus[aria-label="New team"]',
  );
  assert.ok(plus, 'the Teams heading carries a New team button');
  await ui.act(async () => {
    ui.fireEvent.click(plus);
    await Promise.resolve();
  });
  // The sheet the button promises is open on the page it navigated to.
  await ui.waitFor(() =>
    assert.ok(
      [...document.querySelectorAll('.sheet [role="heading"], .sheet h2')].some(
        (node) => node.textContent === 'Create a team',
      ),
      'the create sheet is open',
    ),
  );
  // And the intent is spent: the address left behind is the plain list, so
  // the sheet is not reopened by a later render or by coming back.
  const { location } = store.getSnapshot();
  assert.equal(location.kind, 'teams');
  assert.equal(location.kind === 'teams' ? location.open : 'unset', undefined);
});
