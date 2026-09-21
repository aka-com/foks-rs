import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, useSyncExternalStore } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import type { AgentSnapshot, Item } from '../src/model';
import type { ItemRequest } from '../src/bridge';
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
});
test.after(async () => vite.close());

async function mount(accessNow?: () => number) {
  const { ItemsScreen } = (await vite.ssrLoadModule(
    '/src/screens/items-screen.tsx',
  )) as typeof import('../src/screens/items-screen');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const { LocationStore, INITIAL_STATE } = (await vite.ssrLoadModule(
    '/src/location.ts',
  )) as typeof import('../src/location');
  const { OverlayProvider } = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives');
  const { ToastProvider, ToastController } = (await vite.ssrLoadModule(
    '/kit/toasts.tsx',
  )) as typeof import('../kit/toasts');
  const toasts = new ToastController();
  const locations = new LocationStore({
    ...INITIAL_STATE,
    location: { kind: 'all' },
  });
  const copies: ItemRequest[] = [];
  const downloads: ItemRequest[] = [];
  const reveals: Item[] = [];
  const deletions: Item[] = [];
  const failures: unknown[] = [];
  const folders: { storeId: string; path: string }[] = [];
  const teamInfo: string[] = [];
  const bridge = {
    ...mockBridge(FIXTURE),
    copyItemValue: async (request: ItemRequest) => {
      copies.push(request);
      return { ok: true as const };
    },
    downloadFile: async (request: ItemRequest) => {
      downloads.push(request);
      return { saved: true };
    },
  };
  const backgroundRef = { current: document.getElementById('root')! };
  function Screen({ snapshot }: { snapshot: AgentSnapshot }) {
    const state = useSyncExternalStore(
      locations.subscribe,
      locations.getSnapshot,
    );
    return createElement(ItemsScreen, {
      snapshot,
      bridge,
      state,
      locations,
      accessNow,
      onReveal: (item: Item) => reveals.push(item),
      onDelete: (item: Item) => deletions.push(item),
      onNew: () => {},
      onResume: async () => {},
      onSettings: () => {},
      onNewFolder: (storeId: string, path: string) =>
        folders.push({ storeId, path }),
      onTeamInfo: (storeId: string) => teamInfo.push(storeId),
      onCommandError: (error: unknown) => failures.push(error),
    });
  }
  const view = (snapshot: AgentSnapshot, blocking = false) =>
    createElement(OverlayProvider, {
      backgroundRef,
      portalRoot: document.getElementById('overlays')!,
      blocking,
      children: createElement(ToastProvider, {
        controller: toasts,
        children: createElement(Screen, { snapshot }),
      }),
    });
  const rendered = ui.render(view(FIXTURE));
  return {
    locations,
    copies,
    downloads,
    reveals,
    deletions,
    failures,
    folders,
    teamInfo,
    snapshot: FIXTURE,
    update: (snapshot: AgentSnapshot, blocking = false) =>
      rendered.rerender(view(snapshot, blocking)),
  };
}

function row(name: string): HTMLElement {
  const row = [...document.querySelectorAll<HTMLElement>('.lpane .row')].find(
    (element) => element.querySelector('.nm')?.textContent === name,
  );
  assert.ok(row);
  return row;
}

test('item menus use the existing read actions and restore keyboard focus', async () => {
  const mounted = await mount();
  const password = row('github.com');
  ui.fireEvent.contextMenu(password, { clientX: 120, clientY: 80 });
  assert.equal(mounted.locations.getSnapshot().selection, null);
  assert.equal(mounted.copies.length, 0);
  assert.equal(ui.screen.queryByRole('menuitem', { name: 'Download' }), null);
  ui.fireEvent.click(ui.screen.getByRole('menuitem', { name: 'Copy' }));
  assert.deepEqual(mounted.copies, [
    { storeId: 'acct:personal', path: '/logins/github.com', version: 9 },
  ]);
  assert.equal(ui.screen.queryByRole('menu'), null);
  await ui.waitFor(() => assert.ok(document.activeElement === password));

  ui.fireEvent.keyDown(password, { key: 'F10', shiftKey: true });
  ui.fireEvent.click(ui.screen.getByRole('menuitem', { name: 'Reveal' }));
  assert.equal(mounted.reveals[0]?.path, '/logins/github.com');
  assert.deepEqual(mounted.locations.getSnapshot().selection, {
    store: 'acct:personal',
    path: '/logins/github.com',
  });

  ui.fireEvent.contextMenu(row('passport-scan.pdf'));
  assert.equal(ui.screen.queryByRole('menuitem', { name: 'Copy' }), null);
  ui.fireEvent.click(ui.screen.getByRole('menuitem', { name: 'Download' }));
  assert.equal(mounted.downloads[0]?.path, '/documents/passport-scan.pdf');
  assert.equal(mounted.failures.length, 0);
});

test('details and deletion target the context item, not the current selection', async () => {
  const mounted = await mount();
  ui.fireEvent.click(row('github.com'));
  ui.fireEvent.contextMenu(row('passport-scan.pdf'));
  ui.fireEvent.click(ui.screen.getByRole('menuitem', { name: 'Delete' }));
  assert.equal(mounted.deletions[0]?.path, '/documents/passport-scan.pdf');
  assert.equal(
    mounted.locations.getSnapshot().selection?.path,
    '/logins/github.com',
  );
  assert.equal(ui.screen.queryByRole('menu'), null);

  ui.fireEvent.contextMenu(row('passport-scan.pdf'));
  ui.fireEvent.click(ui.screen.getByRole('menuitem', { name: 'Show details' }));
  assert.equal(
    mounted.locations.getSnapshot().selection?.path,
    '/documents/passport-scan.pdf',
  );
});

test('an open menu rechecks changed write permission and modal blocking dismisses it', async () => {
  const mounted = await mount();
  ui.fireEvent.contextMenu(row('production-token'));
  const snapshot = {
    ...mounted.snapshot,
    parties: mounted.snapshot.parties.map((party) =>
      party.store === 'team:eng' && party.label === 'you'
        ? { ...party, locally_manageable: false }
        : party,
    ),
  };
  mounted.update(snapshot);
  const deletion = ui.screen.getByRole('menuitem', { name: 'Delete' });
  assert.equal(deletion.getAttribute('aria-disabled'), 'true');
  assert.ok(deletion.getAttribute('aria-describedby'));
  ui.fireEvent.click(deletion);
  assert.equal(mounted.deletions.length, 0);
  assert.ok(ui.screen.getByRole('menu'));
  mounted.update(snapshot, true);
  assert.equal(ui.screen.queryByRole('menu'), null);
  mounted.update(snapshot);
  assert.equal(ui.screen.queryByRole('menu'), null);
});

test('Escape and outside clicks dismiss menus without acting', async () => {
  const mounted = await mount();
  const password = row('github.com');
  password.focus();
  ui.fireEvent.keyDown(password, { key: 'ContextMenu' });
  ui.fireEvent.keyDown(document, { key: 'Escape' });
  assert.equal(ui.screen.queryByRole('menu'), null);
  assert.ok(document.activeElement === password);
  ui.fireEvent.contextMenu(password);
  ui.fireEvent.pointerDown(document.body);
  assert.equal(ui.screen.queryByRole('menu'), null);
  assert.equal(
    mounted.copies.length + mounted.reveals.length + mounted.deletions.length,
    0,
  );
});

test('read actions recheck a lease that expires while the menu is open', async () => {
  let now = Date.now() / 1000;
  const mounted = await mount(() => now);
  const lease = mounted.snapshot.servers.find(
    (server) => server.id === 'acme',
  )!.compatibility;
  assert.equal(lease.status, 'required');
  if (lease.status !== 'required') return;
  ui.fireEvent.contextMenu(row('production-token'));
  const copy = ui.screen.getByRole('menuitem', { name: 'Copy' });
  assert.equal(copy.getAttribute('aria-disabled'), null);
  now = lease.expiresAt + 1;
  ui.fireEvent.click(copy);
  assert.equal(mounted.copies.length, 0);
  assert.ok(ui.screen.getByRole('alert'));
});

test('vault and team roots expose scoped folder creation and team info without selection changes', async () => {
  const mounted = await mount();
  const vault = document.querySelector<HTMLElement>(
    '[data-folder-store="acct:personal"][data-folder-path="/"]',
  )!;
  ui.fireEvent.contextMenu(vault);
  assert.equal(
    ui.screen.queryByRole('menuitem', { name: 'Delete folder' }),
    null,
  );
  assert.equal(ui.screen.queryByRole('menuitem', { name: 'Team info' }), null);
  ui.fireEvent.click(ui.screen.getByRole('menuitem', { name: 'New folder' }));
  assert.deepEqual(mounted.folders, [{ storeId: 'acct:personal', path: '/' }]);
  const before = mounted.locations.getSnapshot();
  const team = document.querySelector<HTMLElement>(
    '[data-folder-store="team:eng"][data-folder-path="/"] .fselect',
  )!;
  ui.fireEvent.keyDown(team, { key: 'F10', shiftKey: true });
  assert.ok(ui.screen.getByRole('separator'));
  ui.fireEvent.click(ui.screen.getByRole('menuitem', { name: 'Team info' }));
  assert.deepEqual(mounted.teamInfo, ['team:eng']);
  assert.equal(mounted.locations.getSnapshot(), before);
});

test('tree and content folders target their own path and explain unsupported deletion', async () => {
  const mounted = await mount();
  const folder = document.querySelector<HTMLElement>(
    '.tpane [data-folder-store="acct:personal"][data-folder-path="/logins"] .fselect',
  )!;
  ui.fireEvent.contextMenu(folder);
  const deletion = ui.screen.getByRole('menuitem', { name: 'Delete folder' });
  assert.equal(deletion.getAttribute('aria-disabled'), 'true');
  assert.match(
    document.getElementById(deletion.getAttribute('aria-describedby')!)!
      .textContent,
    /not supported by the desktop agent/,
  );
  ui.fireEvent.click(ui.screen.getByRole('menuitem', { name: 'New folder' }));
  assert.deepEqual(mounted.folders, [
    { storeId: 'acct:personal', path: '/logins' },
  ]);
  ui.act(() => mounted.locations.setFolder('acct:personal|/'));
  const content = document.querySelector<HTMLElement>(
    '.lpane [data-folder-path="/logins"]',
  )!;
  assert.ok(content);
  ui.fireEvent.keyDown(content, { key: 'ContextMenu' });
  ui.fireEvent.click(ui.screen.getByRole('menuitem', { name: 'New folder' }));
  assert.equal(mounted.folders[1].path, '/logins');
  ui.fireEvent.keyDown(content, { key: 'ContextMenu' });
  ui.fireEvent.keyDown(document, { key: 'Escape' });
  assert.equal(ui.screen.queryByRole('menu'), null);
  assert.equal(document.activeElement, content);
});

test('grid cards keep item selection, quick actions, and keyboard context menus', async () => {
  const mounted = await mount();
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Grid view' }));
  const password = row('github.com');
  assert.ok(password.closest('.files-grid'));
  ui.fireEvent.keyDown(password, { key: ' ' });
  assert.equal(password.getAttribute('aria-pressed'), 'true');
  ui.fireEvent.click(
    ui.within(password).getByRole('button', { name: 'Copy github.com' }),
  );
  assert.equal(mounted.copies[0]?.path, '/logins/github.com');
  ui.fireEvent.keyDown(password, { key: 'F10', shiftKey: true });
  ui.fireEvent.click(ui.screen.getByRole('menuitem', { name: 'Reveal' }));
  assert.equal(mounted.reveals[0]?.path, '/logins/github.com');
  await ui.waitFor(() => assert.ok(document.activeElement === password));
  const file = row('passport-scan.pdf');
  ui.fireEvent.contextMenu(file);
  ui.fireEvent.click(ui.screen.getByRole('menuitem', { name: 'Download' }));
  assert.equal(mounted.downloads[0]?.path, '/documents/passport-scan.pdf');
  assert.equal(
    mounted.locations.getSnapshot().selection?.path,
    '/logins/github.com',
  );
  ui.fireEvent.contextMenu(file);
  ui.fireEvent.click(ui.screen.getByRole('menuitem', { name: 'Show details' }));
  assert.equal(
    mounted.locations.getSnapshot().selection?.path,
    '/documents/passport-scan.pdf',
  );
});

test('grid folder menus preserve their scope for right click and Shift+F10', async () => {
  const mounted = await mount();
  ui.act(() => mounted.locations.setFolder('acct:personal|/'));
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Grid view' }));
  const folder = row('logins');
  for (const open of [
    () => ui.fireEvent.contextMenu(folder),
    () => ui.fireEvent.keyDown(folder, { key: 'F10', shiftKey: true }),
  ]) {
    open();
    assert.equal(
      ui.screen
        .getByRole('menuitem', { name: 'Delete folder' })
        .getAttribute('aria-disabled'),
      'true',
    );
    ui.fireEvent.click(ui.screen.getByRole('menuitem', { name: 'New folder' }));
    assert.equal(mounted.locations.getSnapshot().folder, 'acct:personal|/');
  }
  assert.deepEqual(mounted.folders, [
    { storeId: 'acct:personal', path: '/logins' },
    { storeId: 'acct:personal', path: '/logins' },
  ]);
});
