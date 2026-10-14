/**
 * Vault Trash in the running shell: Delete moves an item or a folder into
 * Trash with Undo, the Trash view restores entries (as a copy when the
 * original path is taken), and only entries in Trash are deleted
 * permanently.
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

/** The app open on the personal vault's GitHub login, details showing. */
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
  await rendered.findByRole('button', { name: 'Move to Trash' });
  const paths = async () =>
    (await base.listCatalog()).items
      .filter((item) => item.store === 'acct:personal')
      .map((item) => item.path);
  return { rendered, store, base, paths };
}

function toasts(): string {
  return document.querySelector('.toasts')?.textContent ?? '';
}

async function trashGithub(rendered: RenderResult) {
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Move to Trash' }));
  });
  await ui.waitFor(() => assert.match(toasts(), /Moved github\.com to Trash/));
}

function trashRow(name: string): HTMLElement {
  const row = [
    ...document.querySelectorAll<HTMLElement>('.trash-view .row'),
  ].find((element) => element.querySelector('.nm')?.textContent === name);
  assert.ok(row, `${name} is listed in Trash`);
  return row;
}

test('Move to Trash removes the item at once and Undo puts it back', async () => {
  const { rendered, paths } = await mount();
  await trashGithub(rendered);
  assert.equal(document.querySelector('[role="alertdialog"]'), null);
  // Trash, the owners' Trash folder, and the entry.
  const moved = (await paths()).filter(
    (path) => path === '/.trash' || path.startsWith('/.trash/'),
  );
  assert.equal(moved.length, 3);
  assert.ok(
    moved.some((path) =>
      /^\/\.trash\/owner\/\d{8}T\d{6}~%2Flogins%2Fgithub\.com$/.test(path),
    ),
  );
  assert.ok(!(await paths()).includes(PATH));
  // Trash is not an ordinary folder in the tree.
  assert.equal(
    document.querySelector('.tpane [data-folder-path="/.trash"]'),
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

test('⌘Z undoes the last move to Trash while its toast is offered', async () => {
  const { rendered, paths } = await mount();
  await trashGithub(rendered);
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

test('the Trash view restores, offers a copy for an occupied path, and deletes permanently', async () => {
  const { rendered, store, base, paths } = await mount();
  await trashGithub(rendered);
  // Another github.com is saved where the old one was.
  await base.createTextItem({
    storeId: 'acct:personal',
    path: PATH,
    value: 'user: new',
  });
  await ui.act(async () => store.setFolder('/.trash'));
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
  const conflict = await rendered.findByRole('dialog');
  assert.match(conflict.textContent ?? '', /\/logins\/github\.com/);
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
  assert.ok((await paths()).includes(PATH));

  // Delete permanently names what it cannot promise.
  const pdf = 'passport-scan.pdf';
  await ui.act(async () => store.setFolder(''));
  await ui.act(async () =>
    store.select({ store: 'acct:personal', path: `/documents/${pdf}` }),
  );
  await ui.act(async () => {
    ui.fireEvent.click(
      await rendered.findByRole('button', { name: 'Move to Trash' }),
    );
  });
  await ui.waitFor(() =>
    assert.match(toasts(), /Moved passport-scan\.pdf to Trash/),
  );
  await ui.act(async () => store.setFolder('/.trash'));
  await ui.waitFor(() => assert.ok(trashRow(pdf)));
  ui.fireEvent.click(
    ui.within(trashRow(pdf)).getByRole('button', {
      name: `Delete ${pdf} permanently`,
    }),
  );
  const sheet = await rendered.findByRole('alertdialog');
  assert.match(
    sheet.textContent ?? '',
    /Removes it for everyone\. Earlier encrypted versions may remain on the server\./,
  );
  assert.doesNotMatch(sheet.textContent ?? '', /cannot be undone/);
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(sheet).getByRole('button', { name: 'Delete permanently' }),
    );
  });
  await ui.waitFor(async () =>
    assert.ok(
      !(await paths()).some((path) => path.includes('%2Fpassport-scan.pdf')),
    ),
  );
});

test('a folder moves to Trash whole, restores whole, and Empty Trash removes every entry', async () => {
  const { rendered, store, paths } = await mount();
  const folder = document.querySelector<HTMLElement>(
    '.tpane [data-folder-store="acct:personal"][data-folder-path="/ssh"] .fselect',
  );
  assert.ok(folder);
  ui.fireEvent.contextMenu(folder);
  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('menuitem', { name: 'Move folder to Trash' }),
    );
  });
  await ui.waitFor(() => assert.match(toasts(), /Moved ssh to Trash/));
  assert.ok(
    (await paths()).some((path) =>
      /^\/\.trash\/owner\/\d{8}T\d{6}~%2Fssh\/id_ed25519$/.test(path),
    ),
  );
  await ui.act(async () => store.setFolder('/.trash'));
  await ui.waitFor(() =>
    assert.match(trashRow('ssh').textContent ?? '', /1 item/),
  );
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(trashRow('ssh')).getByRole('button', { name: 'Restore ssh' }),
    );
  });
  await ui.waitFor(async () =>
    assert.ok((await paths()).includes('/ssh/id_ed25519')),
  );

  await trashGithubFromTrashView(rendered, store);
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Empty Trash…' }));
  const sheet = await rendered.findByRole('alertdialog');
  assert.match(sheet.textContent ?? '', /Permanently delete 1 item\?/);
  assert.equal(
    document.activeElement,
    ui.within(sheet).getByRole('button', { name: 'Cancel' }),
  );
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(sheet).getByRole('button', { name: 'Empty Trash' }),
    );
  });
  await ui.waitFor(async () =>
    assert.deepEqual(
      (await paths()).filter((path) => path.startsWith('/.trash/owner/')),
      [],
    ),
  );
  await ui.waitFor(() => assert.ok(rendered.getByText('Trash is empty')));
});

async function trashGithubFromTrashView(
  rendered: RenderResult,
  store: LocationStore,
) {
  await ui.act(async () => store.setFolder(''));
  await ui.act(async () =>
    store.select({ store: 'acct:personal', path: PATH }),
  );
  await ui.act(async () => {
    ui.fireEvent.click(
      await rendered.findByRole('button', { name: 'Move to Trash' }),
    );
  });
  await ui.waitFor(() => assert.match(toasts(), /Moved github\.com to Trash/));
  await ui.act(async () => store.setFolder('/.trash'));
  await ui.waitFor(() => assert.ok(trashRow('github.com')));
}
