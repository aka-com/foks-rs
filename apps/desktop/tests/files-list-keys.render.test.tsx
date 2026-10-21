/**
 * The Files list as a listbox and the selection around it: one Tab stop,
 * arrow keys, Home/End, type-select and ⌘↑; the store root's folder
 * sections; the kind filter inside a store; clicks outside the details
 * that keep the selection (and unsaved edits); and ⌘⌫ scoped to the list.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import type { Location, LocationState, LocationStore } from '../src/location';
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

async function mount(
  location: Location,
  browsing?: Partial<LocationState>,
): Promise<{ store: LocationStore }> {
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
  const store = new LocationStore({ ...INITIAL_STATE, location, ...browsing });
  ui.render(
    createElement(App, {
      snapshot: FIXTURE,
      bridge: mockBridge(FIXTURE),
      store,
    }),
  );
  await ui.waitFor(() => assert.ok(document.querySelector('.lpane')));
  return { store };
}

function listbox(): HTMLElement {
  const list = document.querySelector<HTMLElement>('.lpane [role="listbox"]');
  assert.ok(list, 'the Files list is a listbox');
  return list;
}

function option(name: string): HTMLElement {
  const node = [
    ...listbox().querySelectorAll<HTMLElement>('[role="option"]'),
  ].find((row) => row.querySelector('.nm')?.textContent === name);
  assert.ok(node, `no option named ${name}`);
  return node;
}

function focusedName(): string | null | undefined {
  return document.activeElement?.querySelector('.nm')?.textContent;
}

test('the list is one Tab stop; the arrows, Home, End and typing move the selection and focus', async () => {
  const { store } = await mount({ kind: 'all' });
  const list = listbox();
  assert.equal(list.getAttribute('aria-label'), 'All Items');
  const options = [...list.querySelectorAll<HTMLElement>('[role="option"]')];
  assert.ok(options.length > 3);
  // Exactly one row takes Tab, and the hover actions take none.
  assert.equal(
    options.filter((row) => row.tabIndex === 0).length,
    1,
    'one row is the Tab stop',
  );
  assert.ok(
    [...list.querySelectorAll<HTMLElement>('.acts button')].every(
      (button) => button.tabIndex === -1,
    ),
  );
  const first = options[0];
  ui.fireEvent.click(first);
  first.focus();
  await ui.waitFor(() =>
    assert.equal(first.getAttribute('aria-selected'), 'true'),
  );
  ui.fireEvent.keyDown(first, { key: 'End' });
  const last = options.at(-1)!;
  await ui.waitFor(() => assert.equal(document.activeElement, last));
  assert.equal(last.getAttribute('aria-selected'), 'true');
  assert.equal(last.tabIndex, 0);
  assert.equal(first.tabIndex, -1);
  ui.fireEvent.keyDown(last, { key: 'Home' });
  await ui.waitFor(() => assert.equal(document.activeElement, first));
  // Type-select jumps to the first row whose name starts with the letters.
  for (const key of 'net')
    ui.fireEvent.keyDown(document.activeElement!, { key });
  await ui.waitFor(() => assert.equal(focusedName(), 'netflix'));
  assert.equal(store.getSnapshot().selection?.path, '/streaming/netflix');
});

test('a store root lists its items under a section per folder, with the selected item showing', async () => {
  const { store } = await mount(
    { kind: 'store', ref: 'team:household' },
    {
      selection: { store: 'team:household', path: '/documents/emergency.pdf' },
      details: true,
    },
  );
  const sections = [
    ...listbox().querySelectorAll<HTMLElement>('.gsec.folder-sec'),
  ].map((section) => section.querySelector('.nm')?.textContent);
  assert.deepEqual(sections, ['documents', 'streaming', 'wifi']);
  assert.equal(option('emergency.pdf').getAttribute('aria-selected'), 'true');
  // A section's label is an option: arrows reach it without selecting it,
  // and Return opens the folder.
  const section = listbox().querySelector<HTMLElement>(
    '.folder-sec[data-folder-path="/streaming"]',
  )!;
  option('emergency.pdf').focus();
  ui.fireEvent.keyDown(option('emergency.pdf'), { key: 'ArrowDown' });
  await ui.waitFor(() => assert.equal(document.activeElement, section));
  assert.equal(store.getSnapshot().selection?.path, '/documents/emergency.pdf');
  ui.fireEvent.keyDown(section, { key: 'Enter' });
  await ui.waitFor(() =>
    assert.equal(store.getSnapshot().folder, '/streaming'),
  );
  await ui.waitFor(() => assert.equal(focusedName(), 'netflix'));
  // ⌘↑ goes back to the enclosing folder, focused on the one just left.
  ui.fireEvent.keyDown(document.activeElement!, {
    key: 'ArrowUp',
    metaKey: true,
  });
  // A store page's root is its own address, with no folder set.
  await ui.waitFor(() => assert.equal(store.getSnapshot().folder, ''));
  await ui.waitFor(() =>
    assert.equal(
      document.activeElement?.getAttribute('data-folder-path'),
      '/streaming',
    ),
  );
});

test('the View menu filters a store by kind, and the list head says so', async () => {
  const { store } = await mount({ kind: 'store', ref: 'acct:personal' });
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'View options' }));
  ui.fireEvent.click(
    await ui.screen.findByRole('menuitemradio', { name: 'Passwords' }),
  );
  await ui.waitFor(() => assert.equal(store.getSnapshot().kind, 'Password'));
  // Still Personal, now only its passwords.
  assert.equal(store.getSnapshot().location.kind, 'store');
  const tiles = [...listbox().querySelectorAll('[role="option"] .tile')];
  assert.ok(tiles.length > 0);
  assert.ok(tiles.every((tile) => tile.classList.contains('pw')));
  const chip = ui.screen.getByRole('button', {
    name: 'Showing passwords only. Show all kinds',
  });
  assert.ok(chip.closest('.list-head'));
  ui.fireEvent.click(chip);
  await ui.waitFor(() => assert.equal(store.getSnapshot().kind, 'All'));
  assert.equal(
    ui.screen.queryByRole('button', {
      name: 'Showing passwords only. Show all kinds',
    }),
    null,
  );
});

test('clicks outside the details keep the selection; a click on empty list space clears it', async () => {
  const { store } = await mount(
    { kind: 'store', ref: 'team:household' },
    {
      selection: { store: 'team:household', path: '/streaming/netflix' },
      details: true,
    },
  );
  const click = (target: Element) => {
    ui.fireEvent.pointerDown(target, { button: 0 });
    ui.fireEvent.mouseDown(target, { button: 0 });
    ui.fireEvent.click(target, { button: 0 });
  };
  click(document.querySelector('.topbar .topsearch input')!);
  click(ui.screen.getByRole('button', { name: 'View options' }));
  ui.fireEvent.keyDown(document, { key: 'Escape' });
  click(document.querySelector('.side')!);
  assert.equal(store.getSnapshot().selection?.path, '/streaming/netflix');
  assert.ok(document.querySelector('.details:not(.details-empty)'));
  click(document.querySelector('.lpane .folder-body')!);
  await ui.waitFor(() => assert.equal(store.getSnapshot().selection, null));
});

test('unsaved edits ask before a click on another row discards them', async () => {
  const { store } = await mount(
    { kind: 'store', ref: 'acct:personal' },
    {
      folder: '/logins',
      selection: { store: 'acct:personal', path: '/logins/github.com' },
      details: true,
    },
  );
  ui.fireEvent.click(await ui.screen.findByRole('button', { name: 'Edit' }));
  const field = await ui.screen.findByRole('textbox', { name: 'User name' });
  ui.fireEvent.change(field, { target: { value: 'someone-else' } });
  // A pointer press outside the panel no longer takes the editor down.
  ui.fireEvent.pointerDown(document.querySelector('.side')!, { button: 0 });
  assert.ok(ui.screen.getByRole('textbox', { name: 'User name' }));
  const other = [
    ...listbox().querySelectorAll<HTMLElement>('[role="option"]'),
  ].find(
    (row) =>
      row.getAttribute('aria-selected') !== 'true' && row.matches('.row.item'),
  );
  assert.ok(other, 'the folder lists another item');
  ui.fireEvent.click(other);
  const alert = await ui.screen.findByRole('alertdialog', {
    name: 'Discard changes?',
  });
  assert.equal(store.getSnapshot().selection?.path, '/logins/github.com');
  ui.fireEvent.click(ui.within(alert).getByRole('button', { name: 'Cancel' }));
  await ui.waitFor(() =>
    assert.equal(ui.screen.queryByRole('alertdialog'), null),
  );
  // The draft is still there.
  assert.equal(
    ui.screen.getByRole('textbox', { name: 'User name' }),
    ui.screen.getByDisplayValue('someone-else'),
  );
});

test('⌘⌫ acts only while the focus is in the list or the details', async () => {
  await mount(
    { kind: 'all' },
    {
      selection: { store: 'acct:personal', path: '/logins/github.com' },
      details: true,
    },
  );
  const chat = document.querySelector<HTMLElement>(
    '.side .nav[data-place="chat"]',
  )!;
  chat.focus();
  ui.fireEvent.keyDown(chat, { key: 'Backspace', metaKey: true });
  assert.equal(ui.screen.queryByRole('alertdialog'), null);
  const row = option('github.com');
  row.focus();
  ui.fireEvent.keyDown(row, { key: 'Backspace', metaKey: true });
  const alert = await ui.screen.findByRole('alertdialog');
  ui.fireEvent.click(ui.within(alert).getByRole('button', { name: 'Cancel' }));
  await ui.waitFor(() =>
    assert.equal(ui.screen.queryByRole('alertdialog'), null),
  );
});

test('⌘⌫ on an item the account may not delete shows the reason instead of the alert', async () => {
  await mount(
    { kind: 'all' },
    {
      selection: { store: 'team:eng', path: '/deploy/production-token' },
      details: true,
    },
  );
  const row = option('production-token');
  row.focus();
  ui.fireEvent.keyDown(row, { key: 'Backspace', metaKey: true });
  await ui.waitFor(() => assert.ok(document.querySelector('.toast')));
  assert.equal(ui.screen.queryByRole('alertdialog'), null);
});

test('Team Info opens in the details column beside the list', async () => {
  await mount({ kind: 'store', ref: 'team:household' });
  const section = listbox().querySelector<HTMLElement>('.folder-sec');
  assert.ok(section);
  ui.fireEvent.contextMenu(section);
  ui.fireEvent.click(
    await ui.screen.findByRole('menuitem', { name: 'Team Info' }),
  );
  const panel = await ui.screen.findByRole('complementary', {
    name: 'Team info',
  });
  // It takes the details column: the list keeps its width and no empty
  // details column is drawn beside it.
  assert.ok(panel.classList.contains('details'));
  assert.ok(document.querySelector('.app.with-details'));
  assert.equal(document.querySelector('.details-empty'), null);
});

test('PageDown and PageUp move a page of rows, stopping at the ends', async () => {
  const { store } = await mount({ kind: 'all' });
  const options = [
    ...listbox().querySelectorAll<HTMLElement>('[role="option"]'),
  ];
  assert.ok(options.length > 10);
  const first = options[0];
  first.focus();
  // Without a layout (jsdom) a page is nine rows.
  ui.fireEvent.keyDown(first, { key: 'PageDown' });
  await ui.waitFor(() => assert.ok(document.activeElement === options[9]));
  assert.equal(options[9].getAttribute('aria-selected'), 'true');
  ui.fireEvent.keyDown(options[9], { key: 'PageDown' });
  await ui.waitFor(() => assert.ok(document.activeElement === options.at(-1)));
  ui.fireEvent.keyDown(options.at(-1)!, { key: 'PageUp' });
  ui.fireEvent.keyDown(document.activeElement!, { key: 'PageUp' });
  await ui.waitFor(() => assert.ok(document.activeElement === first));
  assert.ok(store.getSnapshot().selection);
});

test('in cards, Left and Right step a card and Up and Down a line of cards', async () => {
  window.localStorage.setItem('filesView', 'grid');
  await mount({ kind: 'all' });
  const grid = document.querySelector<HTMLElement>('.lpane .files-grid');
  assert.ok(grid);
  const columns = Number(
    /repeat\((\d+)/.exec(grid.style.gridTemplateColumns)?.[1],
  );
  assert.ok(columns > 1, `the grid has ${columns} columns`);
  const options = [
    ...listbox().querySelectorAll<HTMLElement>('[role="option"]'),
  ];
  const at = (index: number) => options[index];
  at(0).focus();
  ui.fireEvent.keyDown(at(0), { key: 'ArrowRight' });
  await ui.waitFor(() => assert.ok(document.activeElement === at(1)));
  ui.fireEvent.keyDown(at(1), { key: 'ArrowDown' });
  await ui.waitFor(() => assert.ok(document.activeElement === at(1 + columns)));
  ui.fireEvent.keyDown(at(1 + columns), { key: 'ArrowLeft' });
  await ui.waitFor(() => assert.ok(document.activeElement === at(columns)));
  ui.fireEvent.keyDown(at(columns), { key: 'ArrowUp' });
  await ui.waitFor(() => assert.ok(document.activeElement === at(0)));
  // Up from the first line stays put.
  ui.fireEvent.keyDown(at(0), { key: 'ArrowUp' });
  assert.ok(document.activeElement === at(0));
});
