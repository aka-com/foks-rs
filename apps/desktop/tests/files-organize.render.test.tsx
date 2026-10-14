/**
 * Organising Files in the running shell: the move sheet's folder tree and
 * its New folder, several rows chosen with ⌘-click, Shift-click or ⌘A and
 * moved or sent to Trash one request at a time with per-item outcomes, and
 * rows dragged onto a folder of the tree.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import type { Bridge, MoveItemRequest } from '../src/bridge';
import type { LocationStore } from '../src/location';
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
  delete (document as { elementFromPoint?: unknown }).elementFromPoint;
});
test.after(async () => vite.close());

const VAULT = 'acct:personal';

/**
 * The app open on the personal vault's /logins folder, or on one item's
 * details. `decorate` replaces bridge calls; `moves` records every move
 * request that reaches the mock.
 */
async function mount({
  decorate = () => ({}),
  selection,
}: {
  decorate?: (base: Bridge) => Partial<Bridge>;
  selection?: string;
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
  const base = mockBridge(FIXTURE);
  const moves: MoveItemRequest[] = [];
  const decorated = decorate(base);
  const bridge: Bridge = {
    ...base,
    moveItem: async (request) => {
      moves.push(request);
      return base.moveItem(request);
    },
    ...decorated,
  };
  const store: LocationStore = new LocationStore({
    ...INITIAL_STATE,
    location: { kind: 'store', ref: VAULT },
    folder: '/logins',
    ...(selection
      ? { selection: { store: VAULT, path: selection }, details: true }
      : {}),
  });
  const rendered = ui.render(
    createElement(App, { snapshot: FIXTURE, bridge, store }),
  );
  await ui.waitFor(() => assert.ok(listRow('github.com')));
  const paths = async () =>
    (await base.listCatalog()).items
      .filter((item) => item.store === VAULT)
      .map((item) => item.path);
  return { rendered, store, base, paths, moves };
}

function listRow(name: string): HTMLElement {
  const row = [
    ...document.querySelectorAll<HTMLElement>('.folder-body .row.one'),
  ].find((element) => element.querySelector('.nm')?.textContent === name);
  assert.ok(row, `${name} is listed`);
  return row;
}

function toasts(): string {
  return document.querySelector('.toasts')?.textContent ?? '';
}

function selectionBar(): HTMLElement {
  const bar = document.querySelector<HTMLElement>(
    '[role="toolbar"][aria-label="Selected items"]',
  );
  assert.ok(bar, 'the selection bar is shown');
  return bar;
}

async function chooseBoth(): Promise<void> {
  await ui.act(async () => {
    ui.fireEvent.click(listRow('github.com'), { metaKey: true });
  });
  await ui.act(async () => {
    ui.fireEvent.click(listRow('fastmail.com'), { metaKey: true });
  });
  assert.match(selectionBar().textContent ?? '', /2 selected/);
}

function treeItem(dialog: HTMLElement, name: string): HTMLElement {
  return ui
    .within(dialog)
    .getByRole('treeitem', { name: new RegExp(`^${name}`) });
}

test('the move sheet picks a folder from the tree, creates one inline, and moves the item there', async () => {
  const created: unknown[] = [];
  const { rendered, paths, moves } = await mount({
    selection: '/logins/github.com',
    decorate: (base) => ({
      createFolder: async (request) => {
        created.push(request);
        return base.createFolder(request);
      },
    }),
  });
  ui.fireEvent.click(
    await rendered.findByRole('button', { name: 'Rename or move…' }),
  );
  const dialog = await rendered.findByRole('dialog', {
    name: 'Rename or move',
  });
  const tree = ui.within(dialog).getByRole('tree', {
    name: 'Destination folder in Personal',
  });
  assert.equal(
    treeItem(dialog, 'logins').getAttribute('aria-selected'),
    'true',
  );
  assert.match(treeItem(dialog, 'logins').textContent ?? '', /Current folder/);
  // An empty folder from the catalog is offered; Trash is not.
  assert.ok(treeItem(dialog, 'ssh'));
  assert.equal(tree.querySelector('[data-picker-path="/.trash"]'), null);

  // Arrow keys move the selection; the preview follows.
  ui.fireEvent.keyDown(treeItem(dialog, 'logins'), { key: 'ArrowUp' });
  await ui.waitFor(() =>
    assert.equal(
      treeItem(dialog, 'prod').getAttribute('aria-selected'),
      'true',
    ),
  );
  assert.match(
    dialog.querySelector('.move-preview')?.textContent ?? '',
    /Moves github\.com to \/env\/prod\/github\.com/,
  );

  ui.fireEvent.click(treeItem(dialog, 'env'));
  ui.fireEvent.click(
    ui.within(dialog).getByRole('button', { name: 'New folder' }),
  );
  ui.fireEvent.change(ui.within(dialog).getByLabelText('New folder'), {
    target: { value: 'archive' },
  });
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(dialog).getByRole('button', { name: 'Create' }),
    );
  });
  await ui.waitFor(() =>
    assert.equal(
      treeItem(dialog, 'archive').getAttribute('aria-selected'),
      'true',
    ),
  );
  assert.deepEqual(created, [{ storeId: VAULT, path: '/env/archive' }]);
  assert.match(
    dialog.querySelector('.move-preview')?.textContent ?? '',
    /Moves github\.com to \/env\/archive\/github\.com/,
  );

  await ui.act(async () => {
    ui.fireEvent.click(ui.within(dialog).getByRole('button', { name: 'Move' }));
  });
  await ui.waitFor(() =>
    assert.match(toasts(), /Moved github\.com to \/env\/archive/),
  );
  assert.equal(moves.length, 1);
  assert.equal(moves[0]?.destination, '/env/archive/github.com');
  assert.ok((await paths()).includes('/env/archive/github.com'));
});

test('a typed folder that does not exist is created before the move', async () => {
  const { rendered, paths } = await mount({ selection: '/logins/github.com' });
  ui.fireEvent.click(
    await rendered.findByRole('button', { name: 'Rename or move…' }),
  );
  const dialog = await rendered.findByRole('dialog', {
    name: 'Rename or move',
  });
  ui.fireEvent.click(ui.within(dialog).getByRole('button', { name: 'Path' }));
  ui.fireEvent.change(ui.within(dialog).getByLabelText('Destination path'), {
    target: { value: '/old/logins/gh' },
  });
  assert.match(
    dialog.querySelector('.move-preview')?.textContent ?? '',
    /Creates \/old\/logins and moves github\.com to \/old\/logins\/gh/,
  );
  await ui.act(async () => {
    ui.fireEvent.click(ui.within(dialog).getByRole('button', { name: 'Move' }));
  });
  await ui.waitFor(async () =>
    assert.ok((await paths()).includes('/old/logins/gh')),
  );
});

test('⌘-click chooses rows and Move to… moves them one at a time to the chosen folder', async () => {
  const { rendered, paths, moves } = await mount();
  await chooseBoth();
  ui.fireEvent.click(
    ui.within(selectionBar()).getByRole('button', { name: 'Move to…' }),
  );
  const dialog = await rendered.findByRole('dialog', { name: 'Move 2 items' });
  ui.fireEvent.click(treeItem(dialog, 'ssh'));
  assert.match(
    dialog.querySelector('.move-preview')?.textContent ?? '',
    /Moves 2 items to \/ssh/,
  );
  await ui.act(async () => {
    ui.fireEvent.click(ui.within(dialog).getByRole('button', { name: 'Move' }));
  });
  await ui.waitFor(() => assert.match(toasts(), /Moved 2 items to \/ssh/));
  // In list order, each from the catalog read just before it.
  assert.deepEqual(
    moves.map((move) => move.destination),
    ['/ssh/fastmail.com', '/ssh/github.com'],
  );
  const now = await paths();
  assert.ok(
    now.includes('/ssh/github.com') && now.includes('/ssh/fastmail.com'),
  );
  assert.equal(rendered.queryByRole('dialog'), null);
});

test('Retry failed sends again only the item whose request did not run', async () => {
  let refused = false;
  const { rendered, paths } = await mount({
    decorate: (base) => ({
      moveItem: async (request) => {
        if (request.path === '/logins/github.com' && !refused) {
          refused = true;
          throw {
            code: 'mutation-in-flight',
            message: 'Another change to this account is still running.',
            retryable: true,
            ambiguous: false,
            fatal: false,
          };
        }
        return base.moveItem(request);
      },
    }),
  });
  // A plain click shows fastmail.com; Shift-click extends from it.
  await ui.act(async () => {
    ui.fireEvent.click(listRow('fastmail.com'));
  });
  await ui.act(async () => {
    ui.fireEvent.click(listRow('github.com'), { shiftKey: true });
  });
  assert.match(selectionBar().textContent ?? '', /2 selected/);
  ui.fireEvent.click(
    ui.within(selectionBar()).getByRole('button', { name: 'Move to…' }),
  );
  const picker = await rendered.findByRole('dialog', { name: 'Move 2 items' });
  ui.fireEvent.click(treeItem(picker, 'agents'));
  await ui.act(async () => {
    ui.fireEvent.click(ui.within(picker).getByRole('button', { name: 'Move' }));
  });
  const progress = await rendered.findByRole('dialog', {
    name: 'Moved 1 of 2 items to /agents',
  });
  assert.match(progress.textContent ?? '', /1 failed/);
  assert.match(progress.textContent ?? '', /Another change to this account/);
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(progress).getByRole('button', { name: 'Retry failed' }),
    );
  });
  await ui.waitFor(() => assert.match(toasts(), /Moved 2 items to \/agents/));
  const now = await paths();
  assert.ok(now.includes('/agents/github.com'));
  assert.ok(now.includes('/agents/fastmail.com'));
});

test('⌘A chooses every row and Delete moves each to Trash', async () => {
  const trashed: string[] = [];
  const { paths } = await mount({
    decorate: (base) => ({
      trashItem: async (request) => {
        trashed.push(request.path);
        return base.trashItem(request);
      },
    }),
  });
  await ui.act(async () => {
    ui.fireEvent.keyDown(listRow('github.com'), { key: 'a', metaKey: true });
  });
  assert.match(selectionBar().textContent ?? '', /2 selected/);
  await ui.act(async () => {
    ui.fireEvent.keyDown(listRow('github.com'), { key: 'Delete' });
  });
  await ui.waitFor(() => assert.match(toasts(), /Moved 2 items to Trash/));
  assert.deepEqual(trashed, ['/logins/fastmail.com', '/logins/github.com']);
  const now = await paths();
  assert.ok(!now.some((path) => path.startsWith('/logins/')));
  // The chosen rows left the list, and the bar with them.
  assert.equal(
    document.querySelector('[role="toolbar"][aria-label="Selected items"]'),
    null,
  );
});

test('Escape clears the chosen rows', async () => {
  await mount();
  await chooseBoth();
  await ui.act(async () => {
    ui.fireEvent.keyDown(listRow('github.com'), { key: 'Escape' });
  });
  assert.equal(
    document.querySelector('[role="toolbar"][aria-label="Selected items"]'),
    null,
  );
});

test('dragging chosen rows onto a tree folder moves them there', async () => {
  const { moves, paths } = await mount();
  await chooseBoth();
  const target = document.querySelector<HTMLElement>(
    `.tpane [data-folder-store="${VAULT}"][data-folder-path="/ssh"]`,
  );
  assert.ok(target, 'the tree lists /ssh');
  let under: Element | null = target;
  (document as { elementFromPoint?: unknown }).elementFromPoint = () => under;
  ui.fireEvent.pointerDown(listRow('github.com'), {
    button: 0,
    clientX: 10,
    clientY: 10,
  });
  await ui.act(async () => {
    ui.fireEvent.pointerMove(window, { clientX: 40, clientY: 60 });
  });
  const ghost = document.querySelector('.drag-ghost');
  assert.match(ghost?.textContent ?? '', /Move 2 items to \/ssh/);
  assert.ok(target.classList.contains('drop-ok'));
  // A folder of another vault does not take them.
  under = document.querySelector(
    '.tpane [data-folder-store="team:eng"][data-folder-path="/"]',
  );
  await ui.act(async () => {
    ui.fireEvent.pointerMove(window, { clientX: 40, clientY: 90 });
  });
  assert.match(
    document.querySelector('.drag-ghost')?.textContent ?? '',
    /Items move only within Personal/,
  );
  under = target;
  await ui.act(async () => {
    ui.fireEvent.pointerUp(window, { clientX: 40, clientY: 60 });
  });
  await ui.waitFor(() => assert.match(toasts(), /Moved 2 items to \/ssh/));
  assert.equal(document.querySelector('.drag-ghost'), null);
  assert.deepEqual(
    moves.map((move) => move.destination),
    ['/ssh/fastmail.com', '/ssh/github.com'],
  );
  assert.ok((await paths()).includes('/ssh/github.com'));
});

test('a press without movement is a click, not a drag', async () => {
  const { moves, store } = await mount();
  ui.fireEvent.pointerDown(listRow('github.com'), {
    button: 0,
    clientX: 10,
    clientY: 10,
  });
  ui.fireEvent.pointerUp(window, { clientX: 11, clientY: 10 });
  await ui.act(async () => {
    ui.fireEvent.click(listRow('github.com'));
  });
  assert.equal(moves.length, 0);
  assert.deepEqual(store.getSnapshot().selection, {
    store: VAULT,
    path: '/logins/github.com',
  });
});
