import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, useSyncExternalStore } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import type { AgentSnapshot, Item, TrashEntry } from '../src/model';
import type { CopyFieldRequest, ItemRequest } from '../src/bridge';
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
  // Every copy requested, with the field it asked for; `value` is the whole
  // text through copyItemValue.
  const copies: CopyFieldRequest[] = [];
  const downloads: ItemRequest[] = [];
  const reveals: Item[] = [];
  const deletions: Item[] = [];
  const restores: TrashEntry[] = [];
  const permanent: Item[] = [];
  const failures: unknown[] = [];
  const folders: { storeId: string; path: string }[] = [];
  const teamInfo: string[] = [];
  const base = mockBridge(FIXTURE);
  const bridge = {
    ...base,
    copyItemValue: async (request: ItemRequest) => {
      copies.push({ ...request, field: 'value' });
      return { ok: true as const };
    },
    copyItemField: async (request: CopyFieldRequest) => {
      copies.push(request);
      return base.copyItemField(request);
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
      onTrash: (item: Item) => deletions.push(item),
      onRestore: (entry: TrashEntry) => restores.push(entry),
      onDeletePermanently: (item: Item) => permanent.push(item),
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
    restores,
    permanent,
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
  assert.equal(ui.screen.queryByRole('menuitem', { name: 'Copy value' }), null);
  assert.ok(ui.screen.getByRole('menuitem', { name: 'Copy user name' }));
  ui.fireEvent.click(
    ui.screen.getByRole('menuitem', { name: 'Copy password' }),
  );
  assert.deepEqual(mounted.copies, [
    {
      storeId: 'acct:personal',
      path: '/logins/github.com',
      version: 9,
      field: 'password',
    },
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

  ui.fireEvent.contextMenu(row('DATABASE_URL'));
  assert.equal(
    ui.screen.queryByRole('menuitem', { name: 'Copy password' }),
    null,
  );
  ui.fireEvent.click(ui.screen.getByRole('menuitem', { name: 'Copy value' }));
  assert.equal(mounted.copies[1]?.field, 'value');
  assert.equal(mounted.copies[1]?.path, '/env/prod/DATABASE_URL');

  ui.fireEvent.contextMenu(row('passport-scan.pdf'));
  assert.equal(
    ui.screen.queryAllByRole('menuitem', { name: /^Copy/ }).length,
    0,
  );
  ui.fireEvent.click(ui.screen.getByRole('menuitem', { name: 'Download' }));
  assert.equal(mounted.downloads[0]?.path, '/documents/passport-scan.pdf');
  assert.equal(mounted.failures.length, 0);
});

test('row copies name the field, and a login without it gets a warning', async () => {
  const mounted = await mount();
  const github = row('github.com');
  ui.fireEvent.click(
    ui.within(github).getByRole('button', {
      name: 'Copy user name github.com',
    }),
  );
  await ui.waitFor(() =>
    assert.match(document.body.textContent ?? '', /User name copied/),
  );
  // The guest network stores a network name and a password, no user name.
  ui.fireEvent.click(
    ui.within(row('guest-password')).getByRole('button', {
      name: 'Copy user name guest-password',
    }),
  );
  await ui.waitFor(() =>
    assert.match(
      document.body.textContent ?? '',
      /This item has no user name\./,
    ),
  );
  assert.deepEqual(
    mounted.copies.map(({ path, field }) => [path, field]),
    [
      ['/logins/github.com', 'username'],
      ['/wifi/guest-password', 'username'],
    ],
  );
  assert.equal(mounted.failures.length, 0);
  // A text Document offers its whole value instead.
  assert.ok(
    ui.within(row('DATABASE_URL')).getByRole('button', {
      name: 'Copy value DATABASE_URL',
    }),
  );
  assert.equal(
    ui.within(row('DATABASE_URL')).queryByRole('button', {
      name: 'Copy password DATABASE_URL',
    }),
    null,
  );
});

test('details and Move to Trash target the context item, not the current selection', async () => {
  const mounted = await mount();
  ui.fireEvent.click(row('github.com'));
  ui.fireEvent.contextMenu(row('passport-scan.pdf'));
  ui.fireEvent.click(
    ui.screen.getByRole('menuitem', { name: 'Move to Trash' }),
  );
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
  const deletion = ui.screen.getByRole('menuitem', { name: 'Move to Trash' });
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
    (server) => server.profileName === 'acme',
  )!.compatibility;
  assert.equal(lease.status, 'required');
  if (lease.status !== 'required') return;
  ui.fireEvent.contextMenu(row('production-token'));
  const copy = ui.screen.getByRole('menuitem', { name: 'Copy value' });
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
    ui.screen.queryByRole('menuitem', { name: 'Move folder to Trash' }),
    null,
  );
  assert.equal(ui.screen.queryByRole('menuitem', { name: 'Team info' }), null);
  assert.equal(
    ui.screen.queryByRole('menuitem', { name: 'Invite new user…' }),
    null,
  );
  ui.fireEvent.click(ui.screen.getByRole('menuitem', { name: 'New folder' }));
  assert.deepEqual(mounted.folders, [{ storeId: 'acct:personal', path: '/' }]);
  const before = mounted.locations.getSnapshot();
  const team = document.querySelector<HTMLElement>(
    '[data-folder-store="team:eng"][data-folder-path="/"] .fselect',
  )!;
  ui.fireEvent.keyDown(team, { key: 'F10', shiftKey: true });
  ui.fireEvent.click(ui.screen.getByRole('menuitem', { name: 'Team info' }));
  assert.deepEqual(mounted.teamInfo, ['team:eng']);
  assert.equal(mounted.locations.getSnapshot(), before);
  for (const [label, intent] of [
    ['Add FOKS user…', 'add'],
    ['Add FOKS team…', 'add-team'],
    ['Invite new user…', 'invite'],
  ] as const) {
    ui.fireEvent.contextMenu(team);
    const menu = ui.screen.getByRole('menu');
    assert.equal(ui.within(menu).getAllByRole('separator').length, 2);
    ui.fireEvent.click(ui.within(menu).getByRole('menuitem', { name: label }));
    assert.equal(ui.screen.queryByRole('menu'), null);
    assert.deepEqual(mounted.locations.getSnapshot().location, {
      kind: 'teams',
      ref: 'team:eng',
      open: intent,
    });
  }
});

test('tree and content folders target their own path and move a loaded folder to Trash', async () => {
  const mounted = await mount();
  const loaded = document.querySelector<HTMLElement>(
    '.tpane [data-folder-store="acct:personal"][data-folder-path="/ssh"] .fselect',
  )!;
  ui.fireEvent.contextMenu(loaded);
  ui.fireEvent.click(
    ui.screen.getByRole('menuitem', { name: 'Move folder to Trash' }),
  );
  assert.equal(mounted.deletions[0]?.path, '/ssh');
  assert.equal(mounted.deletions[0]?.kind, 'Folder');
  const folder = document.querySelector<HTMLElement>(
    '.tpane [data-folder-store="acct:personal"][data-folder-path="/logins"] .fselect',
  )!;
  ui.fireEvent.contextMenu(folder);
  // `/logins` is derived from item paths; the catalog lists no folder entry
  // for it, so there is no dirent to move until the vault is refreshed.
  const deletion = ui.screen.getByRole('menuitem', {
    name: 'Move folder to Trash',
  });
  assert.equal(deletion.getAttribute('aria-disabled'), 'true');
  assert.match(
    document.getElementById(deletion.getAttribute('aria-describedby')!)!
      .textContent,
    /Refresh this vault to load the folder/,
  );
  ui.fireEvent.click(deletion);
  assert.equal(mounted.deletions.length, 1);
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
    ui.within(password).getByRole('button', {
      name: 'Copy password github.com',
    }),
  );
  assert.equal(mounted.copies[0]?.path, '/logins/github.com');
  assert.equal(mounted.copies[0]?.field, 'password');
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
        .getByRole('menuitem', { name: 'Move folder to Trash' })
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

test('All items waits for pending inventories and never presents failed reads as an empty vault', async () => {
  const mounted = await mount();
  const pending: AgentSnapshot = {
    ...mounted.snapshot,
    items: [],
    storeInventory: mounted.snapshot.storeInventory.map((entry) => ({
      ...entry,
      status: 'loading',
    })),
  };
  mounted.update(pending);
  assert.ok(ui.screen.getByText('Loading items…'));
  assert.equal(ui.screen.queryByText('Your vault is empty'), null);
  mounted.update({
    ...pending,
    storeInventory: pending.storeInventory.map((entry) => ({
      ...entry,
      status: 'unavailable',
    })),
  });
  assert.ok(ui.screen.getByText('Could not load items. Refresh to try again.'));
  assert.equal(ui.screen.queryByText('Your vault is empty'), null);
  mounted.update({
    ...pending,
    storeInventory: pending.storeInventory.map((entry) => ({
      ...entry,
      status: 'available',
    })),
  });
  assert.ok(ui.screen.getByText('Your vault is empty'));
});

test('a team Trash lists entries by read role and states who can see them', async () => {
  const mounted = await mount();
  const stamp = '20240102T030405';
  const entry = (path: string, read: Item['read'], id: string): Item => ({
    store: 'team:eng',
    path,
    direntId: id.padStart(32, '0'),
    kind: 'Secret',
    size: 12,
    version: 1,
    read,
    write: 'Admin',
  });
  mounted.update({
    ...mounted.snapshot,
    items: [
      ...mounted.snapshot.items,
      entry(`/.trash/owner/${stamp}~%2Flease-2024.pdf`, 'Owner', 'a1'),
      entry(
        `/.trash/member-0/${stamp}~%2Fwifi%2Fguest-password-2025`,
        { role: 'Member', visibility: 0 },
        'a2',
      ),
    ],
  });
  // The entries are not ordinary items or folders.
  assert.equal(
    document.querySelector('.tpane [data-folder-path="/.trash"]'),
    null,
  );
  ui.act(() => mounted.locations.setFolder('team:eng|/.trash'));
  const view = document.querySelector<HTMLElement>('.trash-view');
  assert.ok(view);
  assert.match(view.textContent ?? '', /keep their read access in Trash/);
  const rows = [...view.querySelectorAll<HTMLElement>('.row')];
  assert.deepEqual(
    rows.map((row) => row.querySelector('.nm')?.textContent),
    ['lease-2024.pdf', 'guest-password-2025'],
  );
  assert.match(rows[0].textContent ?? '', /Owners only/);
  assert.doesNotMatch(rows[1].textContent ?? '', /Owners only/);
  assert.match(rows[1].textContent ?? '', /\/wifi/);
  ui.fireEvent.click(rows[0]);
  assert.match(
    view.querySelector('.trash-detail')?.textContent ?? '',
    /Stored at \/\.trash\/owner\/.*Readable by Owner/,
  );
  ui.fireEvent.click(
    ui.within(rows[1]).getByRole('button', {
      name: 'Restore guest-password-2025',
    }),
  );
  assert.equal(mounted.restores[0]?.original, '/wifi/guest-password-2025');
  ui.fireEvent.keyDown(rows[0], { key: 'Delete' });
  assert.equal(
    mounted.permanent[0]?.path,
    `/.trash/owner/${stamp}~%2Flease-2024.pdf`,
  );
});
