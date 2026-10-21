/**
 * The new-item sheet answering for itself.
 *
 * The sheet is not addressable, so a navigation would leave it floating over a
 * page it was never opened on. These drive the shell's own store: an empty
 * sheet closes behind the move, a sheet with something in it asks first, and a
 * sheet whose save is already with the agent refuses the move outright. Escape
 * asks the same question the guard does.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

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

/** Opens the sheet for `kind` from the toolbar's New menu. */
async function openSheet(
  rendered: Awaited<ReturnType<typeof mount>>['rendered'],
  kind: string,
): Promise<void> {
  ui.fireEvent.click(rendered.getByRole('button', { name: 'New' }));
  ui.fireEvent.click(
    await rendered.findByRole('menuitem', { name: `New ${kind}` }),
  );
  await ui.waitFor(() =>
    assert.ok(rendered.getByRole('dialog', { name: `New ${kind}` })),
  );
}

/** Runs the navigation the sheet is asked about. */
async function leave(store: LocationStore): Promise<void> {
  await ui.act(async () => {
    store.navigate({ kind: 'teams' });
    await Promise.resolve();
  });
}

/** The confirmation on screen, or `null` when nothing is up. */
function confirmation(): HTMLElement | null {
  return document.querySelector<HTMLElement>('[role="alertdialog"]');
}

/** A button in the confirmation's own footer, not the sheet's behind it. */
function dialogButton(label: string): HTMLButtonElement {
  const panel = confirmation();
  assert.ok(panel, 'the confirmation is up');
  const found = [
    ...panel.querySelectorAll<HTMLButtonElement>('.alert .btns .btn'),
  ];
  const target = found.find((node) => node.textContent === label);
  assert.ok(target, `the confirmation offers ${label}`);
  return target;
}

test('an empty sheet closes behind the navigation without asking', async () => {
  const { rendered, store } = await mount();
  await openSheet(rendered, 'Password');

  await leave(store);
  assert.equal(confirmation(), null);
  assert.equal(store.getSnapshot().location.kind, 'teams');
  // The sheet does not float over the page the reader landed on.
  await ui.waitFor(() =>
    assert.equal(rendered.queryByText('New Password'), null),
  );
});

test('Files shortcuts create passwords and documents without replacing an open sheet', async () => {
  const { rendered } = await mount();
  assert.equal(
    ui.fireEvent.keyDown(document, { key: 'n', metaKey: true }),
    false,
  );
  const password = await rendered.findByRole('dialog', {
    name: 'New Password',
  });
  ui.fireEvent.keyDown(document, {
    key: 'n',
    metaKey: true,
    shiftKey: true,
  });
  assert.equal(rendered.queryByRole('dialog', { name: 'New Document' }), null);
  ui.fireEvent.click(
    ui.within(password).getByRole('button', { name: 'Cancel' }),
  );
  assert.equal(
    ui.fireEvent.keyDown(document, {
      key: 'N',
      ctrlKey: true,
      shiftKey: true,
    }),
    false,
  );
  await rendered.findByRole('dialog', { name: 'New Document' });
});

test('the native application menu opens the same new-item workflow', async () => {
  let open: ((kind: 'Password' | 'Document') => void) | undefined;
  const { rendered } = await mount({
    native: true,
    onNewItem: async (listener) => {
      open = listener;
      return () => {};
    },
  });
  await ui.waitFor(() => assert.ok(open));
  ui.act(() => open?.('Document'));
  await rendered.findByRole('dialog', { name: 'New Document' });
});

test('the Files folder menu creates a folder in its selected vault path', async () => {
  let request: Parameters<Bridge['createFolder']>[0] | undefined;
  const { rendered } = await mount({
    createFolder: async (next) => {
      request = next;
      return { applied: true };
    },
  });
  const root = document.querySelector<HTMLElement>(
    '[data-folder-store="acct:personal"][data-folder-path="/"] .fselect',
  );
  assert.ok(root);
  ui.fireEvent.contextMenu(root);
  ui.fireEvent.click(rendered.getByRole('menuitem', { name: 'New Folder…' }));
  await rendered.findByRole('dialog', { name: 'New Folder' });
  ui.fireEvent.change(rendered.getByLabelText('Name'), {
    target: { value: 'Projects' },
  });
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Create' }));
  await ui.waitFor(() => assert.ok(request));
  assert.deepEqual(request, {
    storeId: 'acct:personal',
    path: '/Projects',
  });
});

test('discarding a new folder clears its name before the next sheet', async () => {
  const { rendered } = await mount();
  const root = document.querySelector<HTMLElement>(
    '[data-folder-store="acct:personal"][data-folder-path="/"] .fselect',
  );
  assert.ok(root);
  const open = async () => {
    ui.fireEvent.contextMenu(root);
    ui.fireEvent.click(rendered.getByRole('menuitem', { name: 'New Folder…' }));
    return rendered.findByRole('dialog', { name: 'New Folder' });
  };
  const first = await open();
  ui.fireEvent.change(ui.within(first).getByLabelText('Name'), {
    target: { value: 'DiscardMe' },
  });
  ui.fireEvent.click(ui.within(first).getByRole('button', { name: 'Cancel' }));
  await ui.waitFor(() => assert.ok(confirmation()));
  ui.fireEvent.click(dialogButton('Discard'));
  await ui.waitFor(() => assert.equal(confirmation(), null));
  const second = await open();
  assert.equal(
    ui.within(second).getByRole<HTMLInputElement>('textbox', { name: 'Name' })
      .value,
    '',
  );
});

test('a sheet with something in it asks before the navigation', async () => {
  const { rendered, store } = await mount();
  await openSheet(rendered, 'Password');
  ui.fireEvent.change(rendered.getByLabelText('Site'), {
    target: { value: 'example.com' },
  });

  await leave(store);
  const panel = confirmation();
  assert.ok(panel, 'the confirmation is up');
  assert.equal(
    panel.querySelector('.alert h2')?.textContent,
    'Discard new item?',
  );
  assert.equal(
    panel.querySelector('.alert > p')?.textContent,
    'Your new password has not been saved.',
  );
  // Nothing moves, and the draft is still on screen, while the question is open.
  assert.equal(store.getSnapshot().location.kind, 'store');
  assert.equal(
    (rendered.getByLabelText('Site') as HTMLInputElement).value,
    'example.com',
  );
});

test('confirming the discard closes the sheet and makes the move', async () => {
  const { rendered, store } = await mount();
  await openSheet(rendered, 'Password');
  ui.fireEvent.change(rendered.getByLabelText('Site'), {
    target: { value: 'example.com' },
  });

  await leave(store);
  await ui.act(async () => {
    ui.fireEvent.click(dialogButton('Discard'));
    await Promise.resolve();
  });
  assert.equal(store.getSnapshot().location.kind, 'teams');
  await ui.waitFor(() =>
    assert.equal(rendered.queryByText('New Password'), null),
  );
});

test('canceling the discard keeps the draft and the reader in place', async () => {
  const { rendered, store } = await mount();
  await openSheet(rendered, 'Password');
  ui.fireEvent.change(rendered.getByLabelText('Site'), {
    target: { value: 'example.com' },
  });

  await leave(store);
  await ui.act(async () => {
    ui.fireEvent.click(dialogButton('Cancel'));
    await Promise.resolve();
  });
  assert.equal(confirmation(), null);
  assert.equal(store.getSnapshot().location.kind, 'store');
  assert.equal(
    (rendered.getByLabelText('Site') as HTMLInputElement).value,
    'example.com',
  );
});

test('a save already with the agent refuses the navigation', async () => {
  const { rendered, store } = await mount({
    // The write never settles, so the sheet stays in flight.
    createTextItem: () => new Promise(() => {}),
  });
  await openSheet(rendered, 'Password');
  ui.fireEvent.change(rendered.getByLabelText('Site'), {
    target: { value: 'example.com' },
  });
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Create' }));
    await Promise.resolve();
  });

  await leave(store);
  assert.equal(confirmation(), null);
  assert.equal(store.getSnapshot().location.kind, 'store');
  await ui.waitFor(() =>
    assert.ok(rendered.getByText('Wait for the item to finish saving.')),
  );
});

test('Escape on a sheet with something in it asks the same question', async () => {
  const { rendered } = await mount();
  await openSheet(rendered, 'Document');
  const value = rendered.getByLabelText('Value') as HTMLInputElement;
  assert.equal(value.classList.contains('mono'), false);
  ui.fireEvent.change(value, {
    target: { value: '/ssh/id_ed25519' },
  });

  const sheet = document.querySelector<HTMLElement>('.backdrop');
  assert.ok(sheet);
  await ui.act(async () => {
    ui.fireEvent.keyDown(sheet, { key: 'Escape' });
    await Promise.resolve();
  });
  const panel = confirmation();
  assert.ok(panel, 'the confirmation is up');
  assert.equal(
    panel.querySelector('.alert h2')?.textContent,
    'Discard new item?',
  );
  assert.equal(
    panel.querySelector('.alert > p')?.textContent,
    'Your new document has not been saved.',
  );
  // The draft survives the question until it is answered.
  await ui.act(async () => {
    ui.fireEvent.click(dialogButton('Cancel'));
    await Promise.resolve();
  });
  assert.equal(
    (rendered.getByLabelText('Value') as HTMLInputElement).value,
    '/ssh/id_ed25519',
  );

  await ui.act(async () => {
    ui.fireEvent.keyDown(sheet, { key: 'Escape' });
    await Promise.resolve();
  });
  await ui.act(async () => {
    ui.fireEvent.click(dialogButton('Discard'));
    await Promise.resolve();
  });
  await ui.waitFor(() =>
    assert.equal(rendered.queryByText('New Document'), null),
  );
});

test('losing focus masks a plaintext document draft without closing it', async () => {
  const { rendered } = await mount();
  await openSheet(rendered, 'Document');
  const value = rendered.getByLabelText('Value') as HTMLInputElement;
  ui.fireEvent.change(value, { target: { value: 'private draft token' } });

  ui.fireEvent(window, new Event('blur'));

  const concealed = rendered.getByLabelText('Value') as HTMLInputElement;
  assert.equal(concealed.type, 'password');
  assert.equal(concealed.value, 'private draft token');
  assert.ok(rendered.getByText('New Document'));
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Show' }));
  assert.equal(
    (rendered.getByLabelText('Value') as HTMLInputElement).type,
    'text',
  );
});

test('Path sits under the vault selector and disappears when no vault is available', async () => {
  const { rendered, App, appProps } = await mount();
  await openSheet(rendered, 'Password');

  const vault = rendered.getByRole('button', { name: 'Save in vault' });
  assert.equal(rendered.queryByRole('button', { name: 'Advanced' }), null);
  const path = rendered.getByLabelText('Path');
  assert.equal(path.closest('.inset'), vault.closest('.inset'));

  appProps.snapshot = { ...appProps.snapshot, stores: [] };
  rendered.rerender(createElement(App, appProps));
  assert.equal(rendered.queryByLabelText('Path'), null);
});

for (const kind of ['Password', 'Document']) {
  test(`team permissions follow the ${kind.toLowerCase()} fields`, async () => {
    const { rendered } = await mount();
    await openSheet(rendered, kind);
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Save in vault' }));
    ui.fireEvent.click(
      await rendered.findByRole('option', { name: /^Household/ }),
    );

    const sheet = rendered.getByRole('dialog', { name: `New ${kind}` });
    const sections = [...sheet.querySelectorAll('.sec')].map((section) =>
      section.textContent?.trim(),
    );
    const content = sections.findIndex((label) => label?.startsWith(kind));
    const access = sections.indexOf('Access');
    assert.ok(content >= 0, `${kind} section is present`);
    assert.ok(access > content, 'access follows the item fields');
    // Team items default to member-readable and admin-changeable.
    const read = ui.within(sheet).getByLabelText<HTMLSelectElement>('Read');
    const write = ui.within(sheet).getByLabelText<HTMLSelectElement>('Write');
    assert.equal(read.value, 'Member');
    assert.equal(write.value, 'Admin');
    assert.ok(ui.within(sheet).getByLabelText('Read visibility'));
    ui.fireEvent.change(read, { target: { value: 'Owner' } });
    assert.equal(ui.within(sheet).queryByLabelText('Read visibility'), null);
  });
}

test('a new item resumes its typed input after a rail tab switch', async () => {
  const { rendered, store } = await mount();
  await openSheet(rendered, 'Password');
  ui.fireEvent.change(rendered.getByLabelText('Site'), {
    target: { value: 'draft.example' },
  });
  await ui.act(async () => {
    store.navigateTab('teams');
  });
  assert.equal(confirmation(), null);
  assert.equal(rendered.queryByText('New Password'), null);
  await ui.act(async () => {
    store.navigateTab('files');
  });
  await rendered.findByText('New Password');
  assert.equal(
    (rendered.getByLabelText('Site') as HTMLInputElement).value,
    'draft.example',
  );
  assert.equal(window.location.search.includes('draft.example'), false);
  await leave(store);
  ui.fireEvent.click(dialogButton('Discard'));
  await ui.act(async () => {});
  await ui.act(async () => {
    store.navigateTab('files');
  });
  assert.equal(rendered.queryByText('New Password'), null);
  assert.equal(store.getSnapshot().sheet, undefined);
});

test('item rename keeps the selected source version and blocks dismissal during the write', async () => {
  let request: unknown;
  let finish!: () => void;
  const { rendered, store } = await mount({
    moveItem: async (next) => {
      request = next;
      await new Promise<void>((resolve) => {
        finish = resolve;
      });
      return { applied: true };
    },
  });
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const item = FIXTURE.items.find(
    (item) => item.store === 'acct:personal' && item.kind !== 'Folder',
  )!;
  await ui.act(async () => {
    store.select({ store: item.store, path: item.path });
  });
  await itemMenuAction('Rename or Move…');
  const dialog = await rendered.findByRole('dialog', {
    name: 'Rename or Move',
  });
  ui.fireEvent.change(ui.within(dialog).getByLabelText('Path'), {
    target: { value: '/renamed-item' },
  });
  ui.fireEvent.click(ui.within(dialog).getByRole('button', { name: 'Move' }));
  await ui.waitFor(() =>
    assert.deepEqual(request, {
      storeId: item.store,
      path: item.path,
      version: item.version,
      direntId: item.direntId,
      destination: '/renamed-item',
    }),
  );
  ui.fireEvent.keyDown(dialog, { key: 'Escape' });
  assert.ok(rendered.getByRole('dialog', { name: 'Rename or Move' }));
  await ui.act(async () => finish());
  await ui.waitFor(() =>
    assert.equal(
      rendered.queryByRole('dialog', { name: 'Rename or Move' }),
      null,
    ),
  );
});

test('ambiguous move keeps the draft visible and cannot be submitted twice', async () => {
  let calls = 0;
  const { rendered, store } = await mount({
    moveItem: async () => {
      calls++;
      throw {
        code: 'ambiguous',
        message: 'Move outcome is unknown.',
        fatal: false,
        ambiguous: true,
        retryable: false,
      };
    },
  });
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const item = FIXTURE.items.find(
    (item) => item.store === 'acct:personal' && item.kind !== 'Folder',
  )!;
  await ui.act(async () => {
    store.select({ store: item.store, path: item.path });
  });
  await itemMenuAction('Rename or Move…');
  const dialog = await rendered.findByRole('dialog', {
    name: 'Rename or Move',
  });
  ui.fireEvent.change(ui.within(dialog).getByLabelText('Path'), {
    target: { value: '/uncertain' },
  });
  ui.fireEvent.click(ui.within(dialog).getByRole('button', { name: 'Move' }));
  await ui.waitFor(() => assert.equal(calls, 1));
  await ui.waitFor(() =>
    assert.ok(
      ui
        .within(dialog)
        .getByRole('button', { name: 'Move' })
        .hasAttribute('disabled'),
    ),
  );
  ui.fireEvent.click(ui.within(dialog).getByRole('button', { name: 'Move' }));
  assert.equal(calls, 1);
});

test('Command-Backspace asks to delete the selected item, and Return keeps it', async () => {
  const removed: unknown[] = [];
  const { rendered, store } = await mount({
    removeItem: async (next) => {
      removed.push(next);
      return { applied: true };
    },
  });
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const item = FIXTURE.items.find(
    (item) => item.store === 'acct:personal' && item.kind === 'File',
  )!;
  await ui.act(async () => {
    store.select({ store: item.store, path: item.path });
  });
  await rendered.findByRole('button', { name: 'More actions' });
  // A text field keeps the key for itself.
  const search = document.querySelector<HTMLInputElement>('.topbar input')!;
  ui.fireEvent.keyDown(search, { key: 'Backspace', metaKey: true });
  assert.equal(rendered.queryByRole('alertdialog'), null);
  ui.fireEvent.keyDown(document.body, { key: 'Backspace', metaKey: true });
  const dialog = await rendered.findByRole('alertdialog');
  // Cancel is the default: Return closes the alert and deletes nothing.
  assert.equal(
    document.activeElement,
    ui.within(dialog).getByRole('button', { name: 'Cancel' }),
  );
  ui.fireEvent.keyDown(dialog, { key: 'Enter' });
  await ui.waitFor(() =>
    assert.equal(rendered.queryByRole('alertdialog'), null),
  );
  assert.deepEqual(removed, []);
});

test('file deletion uses its selected version and blocks Escape while pending', async () => {
  let request: unknown;
  let finish!: () => void;
  const { rendered, store } = await mount({
    removeItem: async (next) => {
      request = next;
      await new Promise<void>((resolve) => {
        finish = resolve;
      });
      return { applied: true };
    },
  });
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const item = FIXTURE.items.find(
    (item) => item.store === 'acct:personal' && item.kind === 'File',
  )!;
  assert.ok(item);
  await ui.act(async () => {
    store.select({ store: item.store, path: item.path });
  });
  await itemMenuAction('Delete…');
  const dialog = await rendered.findByRole('alertdialog');
  assert.match(
    dialog.textContent ?? '',
    /deleted if it has not been modified by someone else/,
  );
  ui.fireEvent.click(ui.within(dialog).getByRole('button', { name: 'Delete' }));
  await ui.waitFor(() =>
    assert.deepEqual(request, {
      storeId: item.store,
      path: item.path,
      version: item.version,
      direntId: item.direntId,
    }),
  );
  ui.fireEvent.keyDown(dialog, { key: 'Escape' });
  assert.ok(rendered.getByRole('alertdialog'));
  await ui.act(async () => finish());
  await ui.waitFor(() =>
    assert.equal(rendered.queryByRole('alertdialog'), null),
  );
});
