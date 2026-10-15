/**
 * The ⌘K palette in the running shell: the copy the shell runs for ⌘C and
 * the website it opens for ⌘O, with their access check and toasts, the New
 * actions it offers only on a Files page whose store this account can write,
 * and Add a device, which opens the Devices page's chooser or says why the
 * account cannot add one.
 * `search-palette.render.test.tsx` covers the palette itself with stub
 * handlers.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';

import type { Bridge, CopyFieldRequest, ItemRequest } from '../src/bridge';
import type { Location, LocationStore } from '../src/location';
import type { AgentSnapshot } from '../src/model';
import { installDom } from './lib/dom-harness';
import { loadDeferredScreens } from './lib/deferred-screens';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
});

let ui: typeof import('@testing-library/react');
test.before(async () => {
  ui = await import('@testing-library/react');
  await loadDeferredScreens();
});
test.afterEach(() => {
  ui.cleanup();
  window.localStorage.clear();
});

/**
 * The app open at `location`. `lapse` expires the Acme server's compatibility
 * verification, or that of the server profile it names, which stops access
 * to its stores. `plaintext` replaces the stored text of the items it names.
 * `copies` records every field copy that reaches the mock, and `opens` every
 * Open website request.
 */
async function mount({
  location = { kind: 'store', ref: 'acct:personal' },
  lapse = false,
  plaintext = {},
}: {
  location?: Location;
  lapse?: boolean | string;
  plaintext?: Record<string, string>;
} = {}) {
  const { App } = await import('../src/app-root');
  const { FIXTURE } = await import('../src/mock/fixture');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const { LocationStore, INITIAL_STATE } = await import('../src/location');
  const { applyLease } = await import('../src/model/lease');
  const stored: AgentSnapshot = {
    ...FIXTURE,
    plaintext: { ...FIXTURE.plaintext, ...plaintext },
  };
  const fixture: AgentSnapshot =
    lapse === false
      ? stored
      : lapse === true
        ? applyLease(stored, 'lapsed')
        : applyLease(stored, 'lapsed', lapse);
  const base = mockBridge(fixture);
  const copies: CopyFieldRequest[] = [];
  const opens: ItemRequest[] = [];
  const bridge: Bridge = {
    ...base,
    copyItemField: async (request) => {
      copies.push(request);
      return base.copyItemField(request);
    },
    openItemUrl: async (request) => {
      opens.push(request);
      return base.openItemUrl(request);
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
  return { rendered, copies, opens, store };
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

test('⌘O on a Password opens its website through the host', async () => {
  const { mockOpenedSites } = await import('../src/mock/mock-bridge');
  const before = mockOpenedSites.length;
  const { opens } = await mount();
  await openPalette('github');
  assert.match(
    document.querySelector('.pal-hit.sel .pal-keys')?.textContent ?? '',
    /Open website/,
  );
  ui.fireEvent.keyDown(field(), { key: 'o', metaKey: true });
  await ui.waitFor(() => assert.match(toasts(), /Website opened/));
  assert.equal(document.querySelector('.pal'), null);
  assert.deepEqual(opens, [
    { storeId: 'acct:personal', path: '/logins/github.com', version: 9 },
  ]);
  assert.deepEqual(
    mockOpenedSites.slice(before).map(({ url }) => url),
    ['https://github.com/login'],
  );
  // Opening a website counts as using the item.
  await openPalette('');
  assert.equal(resultNames()[0], 'github.com');
});

test('⌘O on a website with no scheme opens it as https', async () => {
  const { mockOpenedSites } = await import('../src/mock/mock-bridge');
  const before = mockOpenedSites.length;
  // The `://` in the redirect parameter is not a scheme.
  await mount({
    plaintext: {
      'acct:personal|/logins/github.com':
        'username: satoshi\npassword: hx7-Qm2!vTe9-pale-orbit\nurl: accounts.example.com/signin?continue=https://mail.example.com/',
    },
  });
  await openPalette('github');
  ui.fireEvent.keyDown(field(), { key: 'o', metaKey: true });
  await ui.waitFor(() => assert.match(toasts(), /Website opened/));
  assert.deepEqual(
    mockOpenedSites.slice(before).map(({ url }) => url),
    ['https://accounts.example.com/signin?continue=https://mail.example.com/'],
  );
});

test('⌘O on a login without a website says so', async () => {
  const { opens } = await mount();
  await openPalette('fastmail');
  ui.fireEvent.keyDown(field(), { key: 'o', metaKey: true });
  await ui.waitFor(() => assert.match(toasts(), /This item has no website\./));
  assert.equal(opens.length, 1);
});

test('⌘O on a login whose website is not http or https says so', async () => {
  const { opens } = await mount({
    plaintext: {
      'acct:personal|/logins/github.com':
        'username: satoshi\npassword: hx7-Qm2!vTe9-pale-orbit\nurl: ftp://files.example.com',
    },
  });
  await openPalette('github');
  ui.fireEvent.keyDown(field(), { key: 'o', metaKey: true });
  await ui.waitFor(() =>
    assert.match(
      toasts(),
      /This item's website is not an http or https address\./,
    ),
  );
  assert.doesNotMatch(toasts(), /Website opened/);
  assert.equal(opens.length, 1);
});

test('⌘O on an item whose access has stopped warns and opens nothing', async () => {
  const { opens } = await mount({ lapse: 'personal' });
  await openPalette('github');
  ui.fireEvent.keyDown(field(), { key: 'o', metaKey: true });
  await ui.waitFor(() =>
    assert.match(
      toasts(),
      /Compatibility verification for Personal server has expired\./,
    ),
  );
  assert.doesNotMatch(toasts(), /Website opened/);
  assert.deepEqual(opens, []);
});

test('⌘O on a Document does nothing and keeps the palette open', async () => {
  const { opens } = await mount();
  await openPalette('DATABASE_URL');
  assert.equal(resultNames()[0], 'DATABASE_URL');
  assert.doesNotMatch(
    document.querySelector('.pal-hit.sel .pal-keys')?.textContent ?? '',
    /Open website/,
  );
  ui.fireEvent.keyDown(field(), { key: 'o', metaKey: true });
  assert.ok(document.querySelector('.pal'));
  assert.deepEqual(opens, []);
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

test('Add a device opens the Devices chooser and leaves the list address behind', async () => {
  const { rendered, store } = await mount({ location: { kind: 'settings' } });
  await openPalette('add a device');
  assert.equal(resultNames()[0], 'Add a device');
  ui.fireEvent.keyDown(field(), { key: 'Enter' });
  assert.ok(
    await rendered.findByRole('dialog', { name: /Add a device/ }),
    'the chooser opens on the Devices page',
  );
  // The intent is spent: the address is the plain list, at the account.
  assert.deepEqual(store.getSnapshot().location, {
    kind: 'devices',
    store: 'acct:personal',
  });
});

test('Add a device on an account that cannot add one says why', async () => {
  const { rendered, store } = await mount({
    location: { kind: 'settings' },
    lapse: 'personal',
  });
  await openPalette('add a device');
  assert.equal(resultNames()[0], 'Add a device');
  ui.fireEvent.keyDown(field(), { key: 'Enter' });
  await ui.waitFor(() =>
    assert.match(toasts(), /The server check-in has expired\./),
  );
  assert.equal(rendered.queryByRole('dialog', { name: /Add a device/ }), null);
  assert.deepEqual(store.getSnapshot().location, {
    kind: 'devices',
    store: 'acct:personal',
  });
});
