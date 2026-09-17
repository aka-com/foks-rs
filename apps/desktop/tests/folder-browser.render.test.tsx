/**
 * The Files browser: the tree, the breadcrumb header, and the states around
 * their happy path — a store with nothing in it, the "All items" leaf, a
 * multi-folder breadcrumb, and the controls the redesign removed.
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

/** The page header's current (last, unclickable) breadcrumb segment. */
function currentCrumb(): string | undefined {
  return document.querySelector('.loc .crumbs .cur')?.textContent ?? undefined;
}

test('a store with nothing in it is still reachable from the tree', async () => {
  await mount({ kind: 'all' });
  // "Work (Acme)" carries no items in the fixture.
  ui.fireEvent.click(treeRow('Work (Acme)'));
  await ui.waitFor(() => assert.equal(currentCrumb(), 'Work (Acme)'));
  assert.ok(
    document.querySelector('.lpane .empty h2')?.textContent === 'No items yet',
  );
  assert.ok(
    document
      .querySelector('.lpane .empty p')
      ?.textContent?.includes('Work (Acme)'),
  );
  // The tree itself survives showing an empty store: Personal is still there.
  assert.ok(treeRow('Personal'));
});

test('the tree\'s "All items" leaf flattens every store, last under its own heading', async () => {
  await mount({ kind: 'all' });
  const heading = [...document.querySelectorAll('.tpane h6')].at(-1);
  assert.equal(heading?.textContent, 'Everything');
  const all = treeRow('All items');
  assert.equal(all.getAttribute('aria-current'), null);
  ui.fireEvent.click(all);
  await ui.waitFor(() => assert.equal(currentCrumb(), 'All items'));
  assert.equal(treeRow('All items').getAttribute('aria-current'), 'location');
  const rows = document.querySelectorAll('.lpane .body .row');
  assert.ok(rows.length > 1, 'items from more than one store are listed');
});

test('a deep folder draws every ancestor in the breadcrumb, each but the current one clickable', async () => {
  await mount({ kind: 'store', ref: 'acct:personal' }, { folder: '/env/prod' });
  const crumbs = [
    ...document.querySelectorAll('.loc .crumbs > button, .loc .crumbs > .cur'),
  ];
  assert.deepEqual(
    crumbs.map((crumb) => crumb.textContent),
    ['Personal', 'env', 'prod'],
  );
  // The current folder is the page's heading, at the size every other page
  // draws its own title; only the folders above it are buttons.
  assert.equal(crumbs.at(-1)?.tagName, 'H1');
  assert.ok(crumbs.slice(0, -1).every((crumb) => crumb.tagName === 'BUTTON'));

  ui.fireEvent.click(crumbs[1]);
  await ui.waitFor(() => assert.equal(currentCrumb(), 'env'));
  // Landing on the parent folder shows its own child, "prod".
  assert.ok(document.querySelector('.lpane .row.folder'));
});

test('the toolbar carries no view-mode toggle or Details button any more', async () => {
  await mount({ kind: 'all' });
  assert.equal(
    document.querySelector('.toolbar [aria-label="View display mode"]'),
    null,
  );
  assert.equal(document.querySelector('.toolbar [aria-label="Details"]'), null);
  // The kind filter, sort menu and New button are still there.
  assert.ok(
    document.querySelector('.toolbar [aria-label="Filter items by kind"]'),
  );
  assert.ok(document.querySelector('.toolbar .sortwrap'));
  assert.ok(
    [...document.querySelectorAll('.toolbar button')].some((button) =>
      button.textContent?.trim().startsWith('New'),
    ),
  );
});
