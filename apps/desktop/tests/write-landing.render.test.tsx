/**
 * Where a write leaves the user. A renamed or moved item stays selected at
 * its new path with its row focused (its folder opens when the current view
 * does not list it); a created item becomes the selection; a created folder
 * is a folder the list and the sidebar show, and opens.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import type { Bridge } from '../src/bridge';
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

/** Runs an action from the details panel's More menu. */
async function itemMenuAction(label: string): Promise<void> {
  ui.fireEvent.click(
    await ui.screen.findByRole('button', { name: 'More actions' }),
  );
  ui.fireEvent.click(await ui.screen.findByRole('menuitem', { name: label }));
}

async function mount(overrides: Partial<Bridge> = {}) {
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
  const store = new LocationStore({
    ...INITIAL_STATE,
    location: { kind: 'store', ref: 'acct:personal' },
  });
  const appProps = {
    snapshot: FIXTURE,
    bridge: { ...mockBridge(FIXTURE), ...overrides },
    store,
  };
  const rendered = ui.render(createElement(App, appProps));
  await ui.waitFor(() => assert.ok(rendered.getAllByText('Personal').length));
  return { rendered, store, App, appProps };
}

/** The Files list row for `store|path`, if drawn. */
function row(key: string): HTMLElement | undefined {
  return [
    ...document.querySelectorAll<HTMLElement>('.main [data-row-key]'),
  ].find((node) => node.dataset.rowKey === key);
}

/** The sheet titled `name`, if open (a cheap lookup for polling). */
function sheet(name: string): HTMLElement | null {
  return (
    [...document.querySelectorAll<HTMLElement>('[role="dialog"]')].find(
      (dialog) =>
        dialog.getAttribute('aria-label') === name ||
        document.getElementById(dialog.getAttribute('aria-labelledby') ?? '')
          ?.textContent === name,
    ) ?? null
  );
}

async function renameTo(destination: string, button: string): Promise<void> {
  await itemMenuAction('Rename or Move…');
  const dialog = await ui.waitFor(() => {
    const found = sheet('Rename or Move');
    assert.ok(found);
    return found;
  });
  ui.fireEvent.change(ui.within(dialog).getByLabelText('Path'), {
    target: { value: destination },
  });
  const submit = [...dialog.querySelectorAll<HTMLButtonElement>('.btn')].find(
    (node) => node.textContent === button,
  );
  assert.ok(submit, `the sheet offers ${button}`);
  ui.fireEvent.click(submit);
  await ui.waitFor(() =>
    assert.ok(sheet('Rename or Move') === null, 'the sheet closed'),
  );
}

test('a rename keeps the item selected at its new path with its row focused', async () => {
  const { store } = await mount();
  await ui.act(async () => {
    store.select({ store: 'acct:personal', path: '/logins/github.com' });
  });
  await renameTo('/logins/github', 'Rename');
  await ui.waitFor(() =>
    assert.deepEqual(store.getSnapshot().selection, {
      store: 'acct:personal',
      path: '/logins/github',
    }),
  );
  await ui.waitFor(() =>
    assert.ok(
      document.activeElement === row('acct:personal|/logins/github'),
      'the row has the focus',
    ),
  );
  await ui.waitFor(() =>
    assert.match(
      document.querySelector('.toasts')?.textContent ?? '',
      /Renamed github\.com to github/,
    ),
  );
  // The store root still lists the item, so the place does not change.
  assert.equal(store.getSnapshot().folder, '');
});

test('a move out of the listed folders opens the destination folder', async () => {
  const { store } = await mount();
  await ui.act(async () => {
    store.select({ store: 'acct:personal', path: '/logins/github.com' });
  });
  await renameTo('/archive/old/github.com', 'Move');
  await ui.waitFor(() =>
    assert.equal(store.getSnapshot().folder, '/archive/old'),
  );
  assert.deepEqual(store.getSnapshot().selection, {
    store: 'acct:personal',
    path: '/archive/old/github.com',
  });
  await ui.waitFor(() =>
    assert.ok(
      document.activeElement === row('acct:personal|/archive/old/github.com'),
      'the row has the focus',
    ),
  );
  await ui.waitFor(() =>
    assert.match(
      document.querySelector('.toasts')?.textContent ?? '',
      /Moved github\.com to old/,
    ),
  );
});

test('a created item becomes the selection', async () => {
  const { rendered, store } = await mount();
  ui.fireEvent.click(rendered.getByRole('button', { name: 'New' }));
  ui.fireEvent.click(
    await rendered.findByRole('menuitem', { name: 'New Password' }),
  );
  const dialog = await rendered.findByRole('dialog', { name: 'New Password' });
  ui.fireEvent.change(ui.within(dialog).getByLabelText('Site'), {
    target: { value: 'example.org' },
  });
  ui.fireEvent.change(ui.within(dialog).getByLabelText('Password'), {
    target: { value: 'hunter2' },
  });
  ui.fireEvent.click(ui.within(dialog).getByRole('button', { name: 'Create' }));
  await ui.waitFor(() =>
    assert.deepEqual(store.getSnapshot().selection, {
      store: 'acct:personal',
      path: '/logins/example.org',
    }),
  );
  await ui.waitFor(() =>
    assert.ok(
      document.activeElement === row('acct:personal|/logins/example.org'),
      'the row has the focus',
    ),
  );
});

test('a created folder is listed in the sidebar and opens, and New saves into it', async () => {
  const { rendered, store } = await mount();
  ui.fireEvent.click(rendered.getByRole('button', { name: 'New' }));
  ui.fireEvent.click(
    await rendered.findByRole('menuitem', { name: 'New Folder…' }),
  );
  const dialog = await rendered.findByRole('dialog', { name: 'New Folder' });
  ui.fireEvent.change(ui.within(dialog).getByLabelText('Name'), {
    target: { value: 'Projects' },
  });
  ui.fireEvent.click(ui.within(dialog).getByRole('button', { name: 'Create' }));
  await ui.waitFor(() => assert.equal(store.getSnapshot().folder, '/Projects'));
  assert.ok(
    document.querySelector(
      '[data-folder-store="acct:personal"][data-folder-path="/Projects"]',
    ),
    'the sidebar tree has the empty folder',
  );
  await ui.waitFor(() =>
    assert.match(
      document.querySelector('.main .lpane')?.textContent ?? '',
      /This folder is empty\./,
    ),
  );
  // Focus lands on the empty folder's New Password, which saves into it.
  const create = await ui.waitFor(() => {
    const active = document.activeElement as HTMLElement | null;
    assert.equal(active?.textContent, 'New Password');
    return active;
  });
  ui.fireEvent.click(create);
  const next = await ui.waitFor(() => {
    const found = sheet('New Password');
    assert.ok(found);
    return found;
  });
  ui.fireEvent.change(ui.within(next).getByLabelText('Site'), {
    target: { value: 'a.example' },
  });
  assert.ok(ui.within(next).getByDisplayValue('/Projects/a.example'));
});

test('filesListShows follows the list rules for each place', async () => {
  const { filesListShows } = (await vite.ssrLoadModule(
    '/src/screens/scope.ts',
  )) as typeof import('../src/screens/scope');
  const at = (folder: string, extra: Record<string, unknown> = {}) => ({
    location: { kind: 'store', ref: 'acct:personal' } as const,
    folder,
    kind: 'All' as const,
    query: '',
    ...extra,
  });
  const item = (path: string) => ({
    store: 'acct:personal',
    path,
    kind: 'Password',
  });
  // A store root lists its own items and its first-level folders' items.
  assert.equal(filesListShows(at(''), item('/x')), true);
  assert.equal(filesListShows(at(''), item('/a/x')), true);
  assert.equal(filesListShows(at(''), item('/a/b/x')), false);
  // A folder lists its direct items only.
  assert.equal(filesListShows(at('/a'), item('/a/x')), true);
  assert.equal(filesListShows(at('/a'), item('/a/b/x')), false);
  assert.equal(filesListShows(at('/a'), item('/c/x')), false);
  // A search lists every match under its scope; a kind filter narrows.
  assert.equal(filesListShows(at('/a', { query: 'x' }), item('/a/b/x')), true);
  assert.equal(filesListShows(at('/a', { query: 'y' }), item('/a/b/x')), false);
  assert.equal(filesListShows(at('', { kind: 'Document' }), item('/x')), false);
  // All Items lists everything; another store's page lists none of it.
  assert.equal(
    filesListShows(
      { location: { kind: 'all' }, folder: '', kind: 'All', query: '' },
      item('/a/b/c/x'),
    ),
    true,
  );
  assert.equal(
    filesListShows(
      {
        location: { kind: 'store', ref: 'team:eng' },
        folder: '',
        kind: 'All',
        query: '',
      },
      item('/x'),
    ),
    false,
  );
});

test('the move sheet says when the destination changes how the item is listed', async () => {
  const { store } = await mount();
  await ui.act(async () => {
    store.select({ store: 'acct:personal', path: '/env/prod/DATABASE_URL' });
  });
  await itemMenuAction('Rename or Move…');
  const dialog = await ui.waitFor(() => {
    const found = sheet('Rename or Move');
    assert.ok(found);
    return found;
  });
  const hint = 'Under /logins, this item is listed with Passwords.';
  assert.equal(dialog.textContent?.includes(hint), false);
  ui.fireEvent.change(ui.within(dialog).getByLabelText('Path'), {
    target: { value: '/logins/DATABASE_URL' },
  });
  assert.equal(dialog.textContent?.includes(hint), true);
});
