/**
 * The shell's keyboard model: the sidebar as one source list, the window's
 * key equivalents (⌘F, the sidebar toggle), Return in sheets, the palette
 * landing on an item's row, and the focus after a delete.
 *
 * jsdom does not report a Mac, so the off-Mac bindings apply here: Control
 * for ⌘ (Command is accepted too) and Ctrl+Alt+S for ⌃⌘S.
 */
import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, type ReactNode } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import { installDom, settle } from './lib/dom-harness';

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

async function mount(
  overrides: Partial<import('../src/bridge').Bridge> = {},
  initial: Partial<import('../src/location').LocationState> = {},
) {
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
    location: { kind: 'all' },
    ...initial,
  });
  const view = ui.render(
    createElement(App, {
      store,
      snapshot: FIXTURE,
      bridge: { ...mockBridge(FIXTURE), ...overrides },
    }),
  );
  await ui.waitFor(() => assert.ok(document.querySelector('.lpane .row')));
  return { store, view, FIXTURE };
}

function key(target: Element, init: KeyboardEventInit): void {
  ui.act(() => {
    target.dispatchEvent(
      new window.KeyboardEvent('keydown', {
        bubbles: true,
        cancelable: true,
        ...init,
      }),
    );
  });
}

test('the sidebar is one Tab stop that Up and Down walk, and the tree answers Right and Left', async () => {
  await mount();
  const rows = () => [
    ...document.querySelectorAll<HTMLButtonElement>('.side button.nav'),
  ];
  // One row holds the Tab stop: the current place.
  assert.deepEqual(
    rows()
      .filter((row) => row.tabIndex === 0)
      .map((row) => row.dataset.place),
    ['files'],
  );
  const allItems = document.querySelector<HTMLButtonElement>(
    '.side .nav[data-place="files"]',
  )!;
  allItems.focus();
  key(allItems, { key: 'ArrowDown' });
  assert.equal(
    (document.activeElement as HTMLElement).dataset.place,
    'passwords',
  );
  // Focus moves; the place does not change until Return or a click.
  assert.equal(
    rows().find((row) => row.dataset.place === 'passwords')?.tabIndex,
    0,
  );
  assert.equal(allItems.tabIndex, -1);

  const personal = document.querySelector<HTMLButtonElement>(
    '.side-tree [data-folder-store="acct:personal"][data-folder-path="/"] .fselect',
  )!;
  assert.equal(personal.getAttribute('role'), 'treeitem');
  assert.equal(personal.getAttribute('aria-level'), '1');
  assert.equal(personal.getAttribute('aria-expanded'), 'false');
  personal.focus();
  key(personal, { key: 'ArrowRight' });
  await ui.waitFor(() =>
    assert.equal(personal.getAttribute('aria-expanded'), 'true'),
  );
  // Right again steps into the first folder; Left steps back out.
  key(personal, { key: 'ArrowRight' });
  assert.equal(
    (document.activeElement as HTMLElement).getAttribute('aria-level'),
    '2',
  );
  key(document.activeElement!, { key: 'ArrowLeft' });
  assert.equal(document.activeElement, personal);
  key(personal, { key: 'ArrowLeft' });
  await ui.waitFor(() =>
    assert.equal(personal.getAttribute('aria-expanded'), 'false'),
  );
});

test('a sidebar count is read after a comma, not run into the name', async () => {
  await mount();
  const allItems = document.querySelector('.side .nav[data-place="files"]')!;
  assert.ok(allItems.querySelector('.offscreen')?.textContent === ', ');
});

test('⌘F focuses the list search field and the toggle key hides the sidebar', async () => {
  await mount();
  const search = document.querySelector<HTMLInputElement>('.topsearch input')!;
  assert.notEqual(document.activeElement, search);
  key(document.body, { key: 'f', ctrlKey: true });
  assert.equal(document.activeElement, search);
  assert.equal(search.getAttribute('aria-keyshortcuts'), 'Control+F');

  const side = document.querySelector<HTMLElement>('.side')!;
  assert.equal(side.hidden, false);
  key(document.body, { key: 's', ctrlKey: true, altKey: true });
  await ui.waitFor(() => assert.equal(side.hidden, true));
  key(document.body, { key: 's', ctrlKey: true, altKey: true });
  await ui.waitFor(() => assert.equal(side.hidden, false));
});

test('opening an item from the palette opens its folder with the row selected and focused', async () => {
  const { store } = await mount();
  key(document.body, { key: 'k', ctrlKey: true });
  const field = await ui.waitFor(() => {
    const input = document.querySelector<HTMLInputElement>('.pal input');
    assert.ok(input);
    return input;
  });
  ui.fireEvent.change(field, { target: { value: 'github' } });
  key(field, { key: 'Enter' });
  await ui.waitFor(() => {
    const state = store.getSnapshot();
    assert.equal(state.location.kind, 'all');
    assert.equal(state.folder, 'acct:personal|/logins');
    assert.deepEqual(state.selection, {
      store: 'acct:personal',
      path: '/logins/github.com',
    });
  });
  await ui.waitFor(() =>
    assert.equal(
      (document.activeElement as HTMLElement | null)?.dataset.rowKey,
      'acct:personal|/logins/github.com',
    ),
  );
});

test('after a delete the next row is selected and holds the focus', async () => {
  // The mock bridge removes the item, so the refresh drops its row.
  const { store, view } = await mount();
  const keys = [
    ...document.querySelectorAll<HTMLElement>('.lpane [data-row-key]'),
  ]
    .map((row) => row.dataset.rowKey!)
    .filter((rowKey) => !rowKey.endsWith('/'));
  const [first, second] = keys;
  const split = first.indexOf('|');
  await ui.act(async () => {
    store.select({
      store: first.slice(0, split),
      path: first.slice(split + 1),
    });
  });
  key(document.body, { key: 'Backspace', metaKey: true });
  const dialog = await view.findByRole('alertdialog');
  await ui.act(async () => {
    ui.within(dialog).getByRole('button', { name: 'Delete' }).click();
    await Promise.resolve();
  });
  await ui.waitFor(() => assert.equal(view.queryByRole('alertdialog'), null));
  await ui.waitFor(() => {
    const selection = store.getSnapshot().selection;
    assert.equal(`${selection?.store}|${selection?.path}`, second);
  });
  await settle();
  await ui.waitFor(() =>
    assert.equal(
      (document.activeElement as HTMLElement | null)?.dataset.rowKey,
      second,
    ),
  );
});

test('Return in a sheet field presses its enabled default button, and only then', async () => {
  const { SheetDialog } = (await vite.ssrLoadModule(
    '/src/components/sheet.tsx',
  )) as typeof import('../src/components/sheet');
  const { Button } = (await vite.ssrLoadModule(
    '/src/components/button.tsx',
  )) as typeof import('../src/components/button');
  const { OverlayProvider } = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives');
  let presses = 0;
  const sheet = (enabled: boolean): ReactNode =>
    createElement(
      OverlayProvider,
      {
        backgroundRef: { current: document.getElementById('root') },
        portalRoot: document.getElementById('overlays')!,
        children: null,
      },
      createElement(SheetDialog, {
        title: 'Sheet',
        onClose: () => undefined,
        footer: [
          createElement(Button, { key: 'c' }, 'Cancel'),
          createElement(
            Button,
            {
              key: 'p',
              variant: 'primary',
              disabled: !enabled,
              onClick: () => presses++,
            },
            'Create',
          ),
        ],
        children: [
          createElement('input', { key: 'i', 'aria-label': 'Name' }),
          createElement('textarea', { key: 't', 'aria-label': 'Notes' }),
        ],
      }),
    );
  const rendered = ui.render(sheet(false));
  const input = rendered.getByLabelText('Name');
  ui.fireEvent.keyDown(input, { key: 'Enter' });
  assert.equal(presses, 0, 'a disabled default button stays inert');
  rendered.rerender(sheet(true));
  ui.fireEvent.keyDown(input, { key: 'Enter' });
  assert.equal(presses, 1);
  ui.fireEvent.keyDown(rendered.getByLabelText('Notes'), { key: 'Enter' });
  assert.equal(presses, 1, 'a text area keeps Return for a new line');
  ui.fireEvent.keyDown(input, { key: 'Enter', shiftKey: true });
  assert.equal(presses, 1);
});

test('Escape belongs to the page: not from the sidebar, and from blank space it lands on the row', async () => {
  const { store } = await mount(
    {},
    {
      selection: { store: 'acct:personal', path: '/logins/github.com' },
      details: true,
    },
  );
  await ui.waitFor(() => assert.ok(document.querySelector('.details')));
  const allItems = document.querySelector<HTMLButtonElement>(
    '.side .nav[data-place="files"]',
  )!;
  allItems.focus();
  key(allItems, { key: 'Escape' });
  assert.equal(store.getSnapshot().selection?.path, '/logins/github.com');

  // A click on blank space leaves the focus on <body>; Escape there clears
  // the selection and puts the focus on the row that was selected.
  allItems.blur();
  key(document.body, { key: 'Escape' });
  assert.equal(store.getSnapshot().selection, null);
  await ui.waitFor(() =>
    assert.equal(
      (document.activeElement as HTMLElement | null)?.dataset.rowKey,
      'acct:personal|/logins/github.com',
    ),
  );
});

test('VoiceOver text: rows keep their parts apart, a concealed value reads Hidden', async () => {
  await mount(
    {},
    {
      selection: { store: 'acct:personal', path: '/logins/github.com' },
      details: true,
    },
  );
  const row = document.querySelector<HTMLElement>(
    '.lpane [data-row-key="acct:personal|/logins/github.com"]',
  )!;
  assert.match(row.textContent ?? '', /^github\.com, .+, \d/);
  await ui.waitFor(() => assert.ok(document.querySelector('.details .masked')));
  const masked = document.querySelector('.details .masked')!;
  assert.equal(
    masked.querySelector('[aria-hidden="true"]')?.textContent?.[0],
    '•',
  );
  assert.equal(masked.querySelector('.offscreen')?.textContent, 'Hidden');
  // The connection state has a status region of its own.
  assert.ok(
    [...document.querySelectorAll('.side [role="status"]')].some(
      (region) => region.textContent === 'Connected',
    ),
  );
});

test('⌘N in a store that cannot take items says why instead of doing nothing', async () => {
  const { store } = await mount();
  ui.act(() => store.navigate({ kind: 'store', ref: 'team:homelab' }));
  await ui.waitFor(() =>
    assert.equal(store.getSnapshot().location.kind, 'store'),
  );
  key(document.body, { key: 'n', ctrlKey: true });
  await ui.waitFor(() =>
    assert.match(
      document.querySelector('.toast')?.textContent ?? '',
      /Team setup is incomplete/,
    ),
  );
  assert.equal(document.querySelector('.sheet'), null);
});
