/**
 * The ⌘K palette in the running shell: the copy the shell runs for ⌘C, with
 * its access check and toasts, and the New actions it offers only on a Files
 * page whose store this account can write. `search-palette.render.test.tsx`
 * covers the palette itself with stub handlers.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import type { Bridge, CopyFieldRequest } from '../src/bridge';
import type { Location, LocationStore } from '../src/location';
import type { AgentSnapshot } from '../src/model';
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
  window.localStorage.clear();
});
test.after(async () => vite.close());

/**
 * The app open at `location`. `lapse` expires the Acme server's compatibility
 * verification, which stops access to its stores. `copies` records every field copy that
 * reaches the mock.
 */
async function mount({
  location = { kind: 'store', ref: 'acct:personal' },
  lapse = false,
}: {
  location?: Location;
  lapse?: boolean;
} = {}) {
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
  const { applyLease } = (await vite.ssrLoadModule(
    '/src/model/lease.ts',
  )) as typeof import('../src/model/lease');
  const fixture: AgentSnapshot = lapse
    ? applyLease(FIXTURE, 'lapsed')
    : FIXTURE;
  const base = mockBridge(fixture);
  const copies: CopyFieldRequest[] = [];
  const bridge: Bridge = {
    ...base,
    copyItemField: async (request) => {
      copies.push(request);
      return base.copyItemField(request);
    },
  };
  const store: LocationStore = new LocationStore({
    ...INITIAL_STATE,
    location,
  });
  const rendered = ui.render(
    createElement(App, { snapshot: fixture, bridge, store }),
  );
  await ui.waitFor(() => assert.ok(document.querySelector('.topbar')));
  return { rendered, copies };
}

function field(): HTMLInputElement {
  const input = document.querySelector<HTMLInputElement>('.pal-q input');
  assert.ok(input, 'the palette draws its query field');
  return input;
}

async function openPalette(query: string): Promise<void> {
  ui.fireEvent.keyDown(document, { key: 'k', metaKey: true });
  await ui.waitFor(() => assert.ok(document.querySelector('.pal-q input')));
  ui.fireEvent.change(field(), { target: { value: query } });
}

/** The names of the palette's result rows, in order. */
function resultNames(): string[] {
  return [...document.querySelectorAll<HTMLElement>('.pal-hit')].map(
    (row) => row.querySelector('b')?.textContent ?? '',
  );
}

function toasts(): string {
  return document.querySelector('.toasts')?.textContent ?? '';
}

test('⌘C on a Password copies its password and says so', async () => {
  const { copies } = await mount();
  await openPalette('github');
  ui.fireEvent.keyDown(field(), { key: 'c', metaKey: true });
  await ui.waitFor(() => assert.match(toasts(), /Password copied/));
  assert.deepEqual(copies, [
    {
      storeId: 'acct:personal',
      path: '/logins/github.com',
      version: 9,
      field: 'password',
    },
  ]);
});

test('⌘C on an item whose access has stopped warns and copies nothing', async () => {
  const { copies } = await mount({ lapse: true });
  await openPalette('production-token');
  ui.fireEvent.keyDown(field(), { key: 'c', metaKey: true });
  await ui.waitFor(() =>
    assert.match(
      toasts(),
      /Compatibility verification for Acme has expired\. .* items in Engineering\./,
    ),
  );
  assert.doesNotMatch(toasts(), /copied/);
  assert.deepEqual(copies, []);
});

test('New password and Add a document are offered only on a writable Files page', async () => {
  await mount();
  await openPalette('');
  assert.ok(resultNames().includes('New password'));
  assert.ok(resultNames().includes('Add a document'));
  ui.cleanup();

  // Settings is not a Files page.
  await mount({ location: { kind: 'settings' } });
  await openPalette('');
  assert.equal(resultNames().includes('New password'), false);
  assert.ok(resultNames().includes('Lock'));
  ui.cleanup();

  // A vault on a server whose verification has expired cannot be written.
  await mount({ location: { kind: 'store', ref: 'acct:work' } });
  await openPalette('new');
  assert.ok(resultNames().includes('New password'));
  ui.cleanup();
  await mount({ location: { kind: 'store', ref: 'acct:work' }, lapse: true });
  await openPalette('new');
  assert.equal(resultNames().includes('New password'), false);
  assert.equal(resultNames().includes('Add a document'), false);
});

test('Enter on New password closes the palette and opens the new-item sheet', async () => {
  const { rendered } = await mount();
  await openPalette('new password');
  assert.equal(resultNames()[0], 'New password');
  ui.fireEvent.keyDown(field(), { key: 'Enter' });
  await ui.waitFor(() => assert.equal(document.querySelector('.pal'), null));
  assert.ok(
    await rendered.findByRole('dialog', { name: /New password/ }),
    'the sheet opens as the palette closes',
  );
});
