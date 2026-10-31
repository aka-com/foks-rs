/**
 * Vault Trash in the running shell: Move to Trash removes an item at once
 * with Undo (the toast's button or ⌘Z), the Trash view restores entries (as
 * a copy when the original path is taken), permanent deletion from Trash
 * keeps its confirmation, and Empty Trash removes what is in Trash.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import type { RenderResult } from '@testing-library/react';
import type { Bridge } from '../src/bridge';
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
test.afterEach(() => ui.cleanup());
test.after(async () => vite.close());

const PATH = '/logins/github.com';
const PDF = '/documents/passport-scan.pdf';

/** The app open on the personal vault with GitHub's details showing. */
async function mount(decorate: (base: Bridge) => Partial<Bridge> = () => ({})) {
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
  const bridge: Bridge = { ...base, ...decorate(base) };
  const store: LocationStore = new LocationStore({
    ...INITIAL_STATE,
    location: { kind: 'store', ref: 'acct:personal' },
    selection: { store: 'acct:personal', path: PATH },
    details: true,
  });
  const rendered = ui.render(
    createElement(App, { snapshot: FIXTURE, bridge, store }),
  );
  await rendered.findByRole('button', { name: 'More actions' });
  const paths = async () =>
    (await base.listCatalog()).items
      .filter((item) => item.store === 'acct:personal')
      .map((item) => item.path);
  return { rendered, store, base, paths };
}

function toasts(): string {
  return document.querySelector('.toasts')?.textContent ?? '';
}

/** Runs an action from the details panel's More menu. */
async function itemMenuAction(label: string): Promise<void> {
  ui.fireEvent.click(
    await ui.screen.findByRole('button', { name: 'More actions' }),
  );
  ui.fireEvent.click(await ui.screen.findByRole('menuitem', { name: label }));
}

/** Moves the selected item to Trash from the details panel. */
async function trashSelected(name: string): Promise<void> {
  await ui.act(async () => {
    await itemMenuAction('Move to Trash');
  });
  await ui.waitFor(() =>
    assert.ok(toasts().includes(`Moved ${name} to Trash`)),
  );
}

function trashRow(name: string): HTMLElement {
  const row = [
    ...document.querySelectorAll<HTMLElement>('.trash-view .row'),
  ].find((element) => element.querySelector('.nm')?.textContent === name);
  assert.ok(row, `${name} is listed in Trash`);
  return row;
}

async function openTrash(
  rendered: RenderResult,
  store: LocationStore,
): Promise<void> {
  await ui.act(async () => store.setFolder('/.trash'));
  await ui.waitFor(() =>
    assert.ok(rendered.getByRole('heading', { level: 2, name: 'Trash' })),
  );
}

test('Move to Trash removes the item without asking and Undo puts it back', async () => {
  const { paths } = await mount();
  await trashSelected('github.com');
  assert.equal(document.querySelector('[role="alertdialog"]'), null);
  const moved = (await paths()).filter((path) => path.startsWith('/.trash/'));
  assert.ok(
    moved.some((path) =>
      /^\/\.trash\/owner\/\d{8}T\d{6}~%2Flogins%2Fgithub\.com$/.test(path),
    ),
  );
  assert.ok(!(await paths()).includes(PATH));
  // The list no longer shows it, and Trash is not an ordinary folder.
  await ui.waitFor(() =>
    assert.equal(
      document.querySelector(`.lpane [data-row-key="acct:personal|${PATH}"]`),
      null,
    ),
  );
  assert.equal(
    document.querySelector('.lpane [data-folder-path="/.trash"]'),
    null,
  );

  const undo = [
    ...document.querySelectorAll<HTMLButtonElement>('.toast-action'),
  ].find((button) => button.textContent === 'Undo');
  assert.ok(undo);
  await ui.act(async () => {
    ui.fireEvent.click(undo);
  });
  await ui.waitFor(() => assert.match(toasts(), /Restored github\.com/));
  assert.ok((await paths()).includes(PATH));
});

test('⌘Z undoes the last move to Trash once', async () => {
  const { paths } = await mount();
  await trashSelected('github.com');
  await ui.act(async () => {
    ui.fireEvent.keyDown(document.body, { key: 'z', metaKey: true });
  });
  await ui.waitFor(async () => assert.ok((await paths()).includes(PATH)));
  // The Undo is spent: a second ⌘Z changes nothing.
  const before = await paths();
  await ui.act(async () => {
    ui.fireEvent.keyDown(document.body, { key: 'z', metaKey: true });
  });
  assert.deepEqual(await paths(), before);
});

test('the sidebar lists Trash under the vault once something is in it', async () => {
  const { store } = await mount();
  const trashRowInTree = () =>
    document.querySelector<HTMLElement>(
      '.side-tree [data-folder-store="acct:personal"][data-folder-path="/.trash"]',
    );
  await trashSelected('github.com');
  await ui.act(async () => store.setFolder('/.trash'));
  await ui.waitFor(() => assert.ok(trashRowInTree()));
  assert.match(trashRowInTree()?.textContent ?? '', /Trash/);
  assert.equal(
    trashRowInTree()
      ?.querySelector('[role="treeitem"]')
      ?.getAttribute('aria-selected'),
    'true',
  );
});

test('the Trash view restores an entry to where it was', async () => {
  const { rendered, store, paths } = await mount();
  await trashSelected('github.com');
  await openTrash(rendered, store);
  await ui.waitFor(() => assert.ok(trashRow('github.com')));
  assert.match(
    trashRow('github.com').textContent ?? '',
    /\/logins/,
    'the original location is shown',
  );
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(trashRow('github.com')).getByRole('button', {
        name: 'Restore github.com',
      }),
    );
  });
  await ui.waitFor(async () => assert.ok((await paths()).includes(PATH)));
  await ui.waitFor(() => assert.match(toasts(), /Restored github\.com/));
  await ui.waitFor(() =>
    assert.match(rendered.container.textContent ?? '', /Trash Is Empty/),
  );
});

test('restoring onto an occupied path offers a copy beside it, never a replacement', async () => {
  const { rendered, store, base, paths } = await mount();
  await trashSelected('github.com');
  // Another github.com is saved where the old one was.
  await base.createTextItem({
    storeId: 'acct:personal',
    path: PATH,
    value: 'user: new\npassword: other',
  });
  await openTrash(rendered, store);
  await ui.waitFor(() => assert.ok(trashRow('github.com')));
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(trashRow('github.com')).getByRole('button', {
        name: 'Restore github.com',
      }),
    );
  });
  const conflict = await rendered.findByRole('dialog', {
    name: 'github.com Is Already in Use',
  });
  assert.match(
    conflict.textContent ?? '',
    /\/logins\/github\.com \(restored\)/,
  );
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(conflict).getByRole('button', {
        name: 'Restore as github.com (restored)',
      }),
    );
  });
  await ui.waitFor(async () =>
    assert.ok((await paths()).includes(`${PATH} (restored)`)),
  );
  // The item that took the path is still there.
  assert.ok((await paths()).includes(PATH));
});

test('Delete Permanently in Trash asks first and names what it cannot promise', async () => {
  let finish!: () => void;
  const removed: unknown[] = [];
  const { rendered, store, paths } = await mount((base) => ({
    removeItem: async (request) => {
      removed.push(request);
      await new Promise<void>((resolve) => {
        finish = resolve;
      });
      return base.removeItem(request);
    },
  }));
  await ui.act(async () => store.select({ store: 'acct:personal', path: PDF }));
  await trashSelected('passport-scan.pdf');
  await openTrash(rendered, store);
  await ui.waitFor(() => assert.ok(trashRow('passport-scan.pdf')));
  ui.fireEvent.click(
    ui.within(trashRow('passport-scan.pdf')).getByRole('button', {
      name: 'Delete passport-scan.pdf Permanently',
    }),
  );
  const sheet = await rendered.findByRole('alertdialog');
  assert.match(sheet.textContent ?? '', /removes it for everyone/);
  assert.match(
    sheet.textContent ?? '',
    /Earlier encrypted versions may remain/,
  );
  // Cancel is the default button: Return keeps the entry.
  ui.fireEvent.keyDown(sheet, { key: 'Enter' });
  await ui.waitFor(() =>
    assert.equal(rendered.queryByRole('alertdialog'), null),
  );
  assert.deepEqual(removed, []);

  ui.fireEvent.click(
    ui.within(trashRow('passport-scan.pdf')).getByRole('button', {
      name: 'Delete passport-scan.pdf Permanently',
    }),
  );
  const again = await rendered.findByRole('alertdialog');
  ui.fireEvent.click(
    ui.within(again).getByRole('button', { name: 'Delete Permanently' }),
  );
  await ui.waitFor(() => assert.equal(removed.length, 1));
  // The deletion is pending: Escape does not close the dialog.
  ui.fireEvent.keyDown(again, { key: 'Escape' });
  assert.ok(rendered.getByRole('alertdialog'));
  await ui.act(async () => finish());
  await ui.waitFor(() =>
    assert.equal(rendered.queryByRole('alertdialog'), null),
  );
  await ui.waitFor(async () =>
    assert.ok(
      !(await paths()).some((path) => path.includes('%2Fpassport-scan.pdf')),
    ),
  );
});

test('Empty Trash deletes every entry in the vault Trash after confirming', async () => {
  const { rendered, store, paths } = await mount();
  await trashSelected('github.com');
  await ui.act(async () => store.select({ store: 'acct:personal', path: PDF }));
  await rendered.findByRole('heading', { name: 'passport-scan.pdf' });
  await trashSelected('passport-scan.pdf');
  await openTrash(rendered, store);
  await ui.waitFor(() => assert.ok(trashRow('passport-scan.pdf')));
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Empty Trash…' }));
  const sheet = await rendered.findByRole('alertdialog', {
    name: 'Permanently Delete 2 Items?',
  });
  assert.match(sheet.textContent ?? '', /github\.com/);
  assert.match(sheet.textContent ?? '', /passport-scan\.pdf/);
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(sheet).getByRole('button', { name: 'Empty Trash' }),
    );
  });
  await ui.waitFor(() => assert.match(toasts(), /Emptied Trash in Personal/));
  const left = await paths();
  assert.ok(!left.includes(PATH));
  assert.ok(
    !left.some(
      (path) =>
        path.includes('%2Fgithub.com') || path.includes('%2Fpassport-scan.pdf'),
    ),
  );
  await ui.waitFor(() =>
    assert.match(rendered.container.textContent ?? '', /Trash Is Empty/),
  );
});

const CONFLICT = {
  code: 'conflict',
  message: 'The entry changed.',
  retryable: false,
  fatal: false,
  ambiguous: false,
};

test('a restore refused because the entry left Trash says so instead of opening an edit conflict', async () => {
  const { rendered, store } = await mount((base) => ({
    restoreTrashItem: async (request) => {
      // Another session restored it first.
      await base.restoreTrashItem(request);
      throw CONFLICT;
    },
  }));
  await trashSelected('github.com');
  await openTrash(rendered, store);
  await ui.waitFor(() => assert.ok(trashRow('github.com')));
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(trashRow('github.com')).getByRole('button', {
        name: 'Restore github.com',
      }),
    );
  });
  await ui.waitFor(() =>
    assert.match(toasts(), /already restored or removed elsewhere/),
  );
  assert.equal(rendered.queryByText('Item Changed Elsewhere'), null);
  assert.equal(document.querySelector('[role="dialog"]'), null);
});

test('the restore-conflict sheet stays when its copy name was taken meanwhile', async () => {
  const { rendered, store, base } = await mount((base) => ({
    restoreTrashItem: async (request) => {
      // Someone saves the copy's name just before the restore lands.
      if (request.destination.endsWith('(restored)'))
        await base
          .createTextItem({
            storeId: 'acct:personal',
            path: request.destination,
            value: 'user: x\npassword: y',
          })
          .catch(() => undefined);
      return base.restoreTrashItem(request);
    },
  }));
  await trashSelected('github.com');
  await base.createTextItem({
    storeId: 'acct:personal',
    path: PATH,
    value: 'user: new\npassword: other',
  });
  await openTrash(rendered, store);
  await ui.waitFor(() => assert.ok(trashRow('github.com')));
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(trashRow('github.com')).getByRole('button', {
        name: 'Restore github.com',
      }),
    );
  });
  const first = await rendered.findByRole('dialog', {
    name: 'github.com Is Already in Use',
  });
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(first).getByRole('button', {
        name: 'Restore as github.com (restored)',
      }),
    );
  });
  // The sheet for the taken copy name replaces it rather than vanishing.
  await rendered.findByRole('dialog', {
    name: 'github.com (restored) Is Already in Use',
  });
});

test('a permanent deletion refused as a conflict reports the entry gone, not an edit conflict', async () => {
  const { rendered, store } = await mount((base) => ({
    removeItem: async (request) => {
      // Another session deleted it first.
      await base.removeItem(request);
      throw CONFLICT;
    },
  }));
  await ui.act(async () => store.select({ store: 'acct:personal', path: PDF }));
  await trashSelected('passport-scan.pdf');
  await openTrash(rendered, store);
  await ui.waitFor(() => assert.ok(trashRow('passport-scan.pdf')));
  ui.fireEvent.click(
    ui.within(trashRow('passport-scan.pdf')).getByRole('button', {
      name: 'Delete passport-scan.pdf Permanently',
    }),
  );
  const sheet = await rendered.findByRole('alertdialog');
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(sheet).getByRole('button', { name: 'Delete Permanently' }),
    );
  });
  await ui.waitFor(() => assert.match(toasts(), /already deleted elsewhere/));
  assert.equal(rendered.queryByText('Item Changed Elsewhere'), null);
});

test('a folder holding entries this account cannot remove stays in Trash', async () => {
  const hide = (catalog: Awaited<ReturnType<Bridge['listCatalog']>>) => ({
    ...catalog,
    items: catalog.items.map((item) =>
      item.path.endsWith('~%2Fops') ? { ...item, unreadableEntries: 2 } : item,
    ),
  });
  const { rendered, store, base } = await mount((base) => ({
    listCatalog: async (...args) => hide(await base.listCatalog(...args)),
    listProfileCatalog: async (profile) =>
      hide(await base.listProfileCatalog(profile)),
  }));
  await base.createFolder({ storeId: 'acct:personal', path: '/ops' });
  await base.createTextItem({
    storeId: 'acct:personal',
    path: '/ops/runbook',
    value: 'restart',
  });
  const ops = (await base.listCatalog()).items.find(
    (item) => item.path === '/ops',
  )!;
  await base.trashItem({
    storeId: 'acct:personal',
    path: '/ops',
    version: ops.version,
    direntId: ops.direntId,
  });
  // The host selects the next item from a catalog read after this write.
  await base.listCatalog();
  // Moving github.com to Trash reads the vault again, with /ops in Trash.
  await trashSelected('github.com');
  await openTrash(rendered, store);
  await ui.waitFor(() => assert.ok(trashRow('ops')));
  const remove = ui.within(trashRow('ops')).getByRole('button', {
    name: 'Delete ops Permanently',
  });
  assert.ok(remove.hasAttribute('disabled'));
  assert.match(remove.title, /holds 2 entries you cannot remove/);
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Empty Trash…' }));
  const sheet = await rendered.findByRole('alertdialog', {
    name: 'Permanently Delete 1 Item?',
  });
  assert.match(sheet.textContent ?? '', /stays in Trash/);
});
