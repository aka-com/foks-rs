/**
 * The Files browser: the tree, the topbar crumb, and their states — an empty
 * store, the "All items" leaf, a deep folder, the table's columns and sort
 * headers, and the toolbar's controls.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import type { Location, LocationState } from '../src/location';
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

async function mount(location: Location, browsing?: Partial<LocationState>) {
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
  const rendered = ui.render(
    createElement(App, {
      snapshot: FIXTURE,
      bridge: mockBridge(FIXTURE),
      store: new LocationStore({ ...INITIAL_STATE, location, ...browsing }),
    }),
  );
  await ui.waitFor(() => assert.ok(document.querySelector('.tpane')));
  return rendered;
}

/** A tree row's own select button, found by the name it draws. */
function treeRow(name: string): HTMLButtonElement {
  const node = [
    ...document.querySelectorAll<HTMLButtonElement>('.tpane .fselect'),
  ].find((button) => button.querySelector('.nm')?.textContent === name);
  assert.ok(node, `no tree row named ${name}`);
  return node;
}

/** The topbar crumb's last segment: the folder the browser has open. */
function currentTitle(): string | undefined {
  return [...document.querySelectorAll('.topbar .crumbs > span')]
    .at(-1)
    ?.textContent?.replace(/›\s*$/, '')
    .trim();
}

/** The table header's column labels, sort arrow stripped. */
function columns(): string[] {
  return [...document.querySelectorAll('.lpane .hdr > *')]
    .map((cell) => cell.textContent?.replace(' ↓', '').trim() ?? '')
    .filter(Boolean);
}

test('defaults to All items, displaying all stores in one table', async () => {
  await mount({ kind: 'all' });
  assert.equal(currentTitle(), 'All items');
  assert.equal(treeRow('All items').getAttribute('aria-current'), 'location');
  // The page draws no header of its own: the crumb is the title.
  assert.equal(document.querySelector('.lpane .path'), null);
  assert.equal(document.querySelector('.loc h1'), null);
  const rows = document.querySelectorAll('.lpane .body .row');
  assert.ok(rows.length > 1, 'items from more than one store are listed');
  // Items span stores, so Location identifies each item's store.
  assert.deepEqual(columns(), ['Name', 'Kind', 'Location', 'Size']);
  // All items is the Files root, so Back is disabled.
  const back = document.querySelector<HTMLButtonElement>('.rail-back');
  assert.ok(back);
  assert.equal(back.disabled, true);
});

test('All items is the first tree row; stores follow under their headings', async () => {
  await mount({ kind: 'all' });
  const names = [...document.querySelectorAll('.tpane .fselect .nm')].map(
    (node) => node.textContent,
  );
  assert.equal(names[0], 'All items');
  assert.deepEqual(
    [...document.querySelectorAll('.tpane h6')].map((h) => h.textContent),
    ['Vaults', 'Teams'],
  );
  // Store root rows cannot be collapsed and have no disclosure toggle; nested
  // folders with children render a toggle.
  assert.equal(document.querySelector('.tpane .fn.root .twist'), null);
  assert.ok(document.querySelector('.tpane .fn:not(.root) .twist'));
  // Teams are marked with their initials; vaults keep the vault glyph.
  assert.ok(treeRow('Household').querySelector('.av.team'));
  assert.equal(treeRow('Personal').querySelector('.av.team'), null);
  // Item counts appear only on rows that contain items.
  assert.equal(treeRow('Personal').querySelector('.c')?.textContent, '6');
  assert.equal(treeRow('Work (Acme)').querySelector('.c'), null);
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
  assert.equal(treeRow('Homelab').parentElement?.querySelector('.fact'), null);
  ui.fireEvent.click(treeRow('Homelab'));
  await ui.waitFor(() => assert.equal(currentTitle(), 'Homelab'));
  assert.equal(
    document.querySelector('.lpane .empty p')?.textContent,
    'Items stored in this team vault are accessible to team members according to their assigned roles.',
  );
  assert.ok(
    treeRow('Homelab').parentElement?.querySelector(
      '.fact[aria-label="Team settings"]',
    ),
  );
  assert.equal(
    document.querySelector('.toolbar [aria-label="Team settings"]'),
    null,
  );
  assert.equal(
    document.querySelector<HTMLInputElement>('.toolbar .search input')
      ?.placeholder,
    'Search this team',
  );
});

test('a deep folder displays its breadcrumb and omits the Location column', async () => {
  await mount({ kind: 'store', ref: 'acct:personal' }, { folder: '/env/prod' });
  assert.equal(currentTitle(), 'prod');
  assert.deepEqual(columns(), ['Name', 'Kind', 'Size']);
  assert.equal(
    document.querySelector<HTMLInputElement>('.toolbar .search input')
      ?.placeholder,
    'Search this folder',
  );

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

test('the toolbar contains only the kind filter, scoped search, and New button', async () => {
  await mount({ kind: 'all' });
  assert.equal(
    document.querySelector('.toolbar [aria-label="View display mode"]'),
    null,
  );
  assert.equal(document.querySelector('.toolbar [aria-label="Details"]'), null);
  assert.equal(document.querySelector('.toolbar .sortwrap'), null);
  assert.ok(
    document.querySelector('.toolbar [aria-label="Filter items by kind"]'),
  );
  const search = document.querySelector<HTMLInputElement>(
    '.toolbar .search input',
  );
  assert.ok(search);
  assert.equal(search.placeholder, 'Search all items');
  // ⌘K belongs to the global palette; the scoped field shows no badge.
  assert.equal(document.querySelector('.toolbar .search kbd'), null);
  // The toolbar spans both columns: it is a sibling above the split, not a
  // child of the list pane, with the filter in the tree-width cell and the
  // rest over the list.
  const toolbar = document.querySelector('.toolbar');
  assert.ok(toolbar);
  assert.equal(toolbar.closest('.lpane'), null);
  assert.equal(
    toolbar.parentElement?.classList.contains('folder-layout'),
    true,
  );
  assert.ok(toolbar.nextElementSibling?.classList.contains('folder-split'));
  assert.ok(
    toolbar.querySelector(
      '.toolbar-filter [aria-label="Filter items by kind"]',
    ),
  );
  // Search sits immediately before New.
  const controls = [...document.querySelectorAll('.toolbar-rest > *')];
  const searchIndex = controls.findIndex((node) =>
    node.classList.contains('search'),
  );
  assert.ok(
    controls[searchIndex + 1]?.textContent?.trim().startsWith('New'),
    'New follows the search field',
  );
});

test('search scopes to the selected tree row', async () => {
  await mount({ kind: 'all' });
  ui.fireEvent.click(treeRow('Household'));
  await ui.waitFor(() => assert.equal(currentTitle(), 'Household'));
  const search = document.querySelector<HTMLInputElement>(
    '.toolbar .search input',
  );
  assert.ok(search);
  // "github.com" lives in Personal, not Household.
  ui.fireEvent.change(search, { target: { value: 'github' } });
  await ui.waitFor(() =>
    assert.match(
      document.querySelector('.lpane .empty h2')?.textContent ?? '',
      /No items in Household match/,
    ),
  );
  ui.fireEvent.change(search, { target: { value: 'netflix' } });
  await ui.waitFor(() =>
    assert.equal(document.querySelectorAll('.lpane .body .row').length, 1),
  );
});
