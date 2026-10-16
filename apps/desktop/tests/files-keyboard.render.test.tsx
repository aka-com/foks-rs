/**
 * Keyboard use of the Files list, its card grid and the folder tree: each is
 * one tab stop with grid or tree semantics, and the arrow keys move focus,
 * including to rows the virtual window has not mounted yet.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, useSyncExternalStore } from 'react';
import type { AgentSnapshot, Item } from '../src/model';
import { installDom } from './lib/dom-harness';
import { loadTestShell } from './lib/test-shell';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
});

let ui: typeof import('@testing-library/react');
let FIXTURE: AgentSnapshot;
test.before(async () => {
  ui = await import('@testing-library/react');
  ({ FIXTURE } = await import('../src/mock/fixture'));
});
test.afterEach(() => {
  ui.cleanup();
  window.localStorage.removeItem('filesView');
});

async function mount(snapshot: AgentSnapshot = FIXTURE) {
  const { ItemsScreen } = await import('../src/features/files/items-screen');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const { LocationStore, INITIAL_STATE } = await import('../src/location');
  const { OverlayProvider } = await import('../kit/overlay-primitives');
  const { ToastProvider, ToastController } = await import('../kit/toasts');
  const TestShell = await loadTestShell();
  const locations = new LocationStore({
    ...INITIAL_STATE,
    location: { kind: 'all' },
  });
  const settings: string[] = [];
  const bridge = mockBridge(snapshot);
  function Screen() {
    const state = useSyncExternalStore(
      locations.subscribe,
      locations.getSnapshot,
    );
    return createElement(TestShell, {
      snapshot,
      bridge,
      children: createElement(ItemsScreen, {
        state,
        locations,
        onReveal: () => {},
        onTrash: () => {},
        onMoveMany: () => {},
        onNew: () => {},
        onResume: async () => {},
        onSettings: (storeId: string) => settings.push(storeId),
      }),
    });
  }
  ui.render(
    createElement(OverlayProvider, {
      backgroundRef: { current: document.getElementById('root')! },
      portalRoot: document.getElementById('overlays')!,
      blocking: false,
      children: createElement(ToastProvider, {
        controller: new ToastController(),
        children: createElement(Screen),
      }),
    }),
  );
  return { locations, settings };
}

function grid(): HTMLElement {
  return ui.screen.getByRole('grid');
}

/** The grid's entries that are tab stops. */
function tabStops(container: HTMLElement): HTMLElement[] {
  return [...container.querySelectorAll<HTMLElement>('[tabindex="0"]')];
}

function focused(): HTMLElement {
  assert.ok(document.activeElement instanceof HTMLElement);
  return document.activeElement;
}

function named(element: Element): string {
  return element.querySelector('.nm')?.textContent ?? '';
}

/** FIXTURE plus `count` passwords in one folder of the personal vault. */
function crowded(count: number): AgentSnapshot {
  const extra: Item[] = Array.from({ length: count }, (_, index) => ({
    store: 'acct:personal',
    path: `/bulk/entry-${String(index).padStart(3, '0')}`,
    direntId: (0xb000 + index).toString(16).padStart(32, '0'),
    kind: 'Secret',
    size: 20,
    version: 1,
    read: 'Owner',
    write: 'Owner',
  }));
  return { ...FIXTURE, items: [...FIXTURE.items, ...extra] };
}

test('the list is a grid with column headers, row positions and one tab stop', async () => {
  await mount();
  const table = grid();
  const rows = [...table.querySelectorAll<HTMLElement>('.row')];
  assert.ok(rows.length > 1);
  assert.equal(table.getAttribute('aria-rowcount'), String(rows.length + 1));
  assert.equal(table.getAttribute('aria-multiselectable'), 'true');
  assert.deepEqual(
    ui
      .within(table)
      .getAllByRole('columnheader')
      .map((cell) => cell.textContent?.replace(/ [↑↓]/, '')),
    ['Name', 'Kind', 'Location', 'Size', 'Actions'],
  );
  assert.equal(
    ui.within(table).getAllByRole('columnheader')[0].getAttribute('aria-sort'),
    'ascending',
  );
  assert.deepEqual(
    rows.map((row) => row.getAttribute('aria-rowindex')),
    rows.map((_, index) => String(index + 2)),
  );
  for (const row of rows) {
    assert.equal(row.getAttribute('role'), 'row');
    assert.ok(ui.within(row).getAllByRole('gridcell').length >= 4);
  }
  // No row is a button, and the row's own actions are not tab stops.
  assert.equal(table.querySelector('[role="button"]'), null);
  assert.deepEqual(tabStops(table), [rows[0]]);
  for (const button of table.querySelectorAll('.acts button'))
    assert.equal(button.getAttribute('tabindex'), '-1');
  // A row cannot carry aria-haspopup; it names the key for its menu.
  for (const row of rows)
    assert.equal(row.getAttribute('aria-keyshortcuts'), 'Shift+F10');
});

test('arrow keys move between rows and into a row’s own actions', async () => {
  const { locations } = await mount();
  const table = grid();
  const rows = () => [...table.querySelectorAll<HTMLElement>('.row')];
  rows()[0].focus();
  ui.fireEvent.keyDown(focused(), { key: 'ArrowDown' });
  assert.equal(focused(), rows()[1]);
  assert.deepEqual(tabStops(table), [rows()[1]]);
  ui.fireEvent.keyDown(focused(), { key: 'End' });
  assert.equal(focused(), rows().at(-1));
  ui.fireEvent.keyDown(focused(), { key: 'ArrowUp' });
  assert.equal(focused(), rows().at(-2));
  ui.fireEvent.keyDown(focused(), { key: 'Home' });
  const first = rows()[0];
  assert.equal(focused(), first);
  // Right walks the row's actions and Left walks back to the row.
  const actions = [...first.querySelectorAll<HTMLElement>('.acts button')];
  assert.ok(actions.length > 1);
  ui.fireEvent.keyDown(first, { key: 'ArrowRight' });
  assert.equal(focused(), actions[0]);
  ui.fireEvent.keyDown(focused(), { key: 'ArrowRight' });
  assert.equal(focused(), actions[1]);
  ui.fireEvent.keyDown(focused(), { key: 'ArrowLeft' });
  ui.fireEvent.keyDown(focused(), { key: 'ArrowLeft' });
  assert.equal(focused(), first);
  ui.fireEvent.keyDown(first, { key: 'ArrowLeft' });
  assert.equal(focused(), first);
  // Down from an action moves to the next row.
  ui.fireEvent.keyDown(first, { key: 'ArrowRight' });
  ui.fireEvent.keyDown(focused(), { key: 'ArrowDown' });
  assert.equal(focused(), rows()[1]);
  // Enter opens the row's details.
  ui.fireEvent.keyDown(focused(), { key: 'Enter' });
  assert.equal(
    locations.getSnapshot().selection?.path.endsWith(named(rows()[1])),
    true,
  );
  assert.equal(rows()[1].getAttribute('aria-selected'), 'true');
});

test('the arrow keys reach rows outside the virtual window and keep the tab stop mounted', async () => {
  await mount(crowded(200));
  const table = grid();
  const total = Number(table.getAttribute('aria-rowcount')) - 1;
  assert.ok(total > 200);
  const mounted = () => table.querySelectorAll('.row').length;
  assert.ok(mounted() < total, 'the list is windowed');
  table.querySelector<HTMLElement>('.row')!.focus();
  ui.fireEvent.keyDown(focused(), { key: 'End' });
  await ui.waitFor(() =>
    assert.equal(focused().getAttribute('aria-rowindex'), String(total + 1)),
  );
  assert.ok(mounted() < total);
  const last = focused();
  // Scrolling back to the top leaves the focused row mounted, past the
  // window's edge, as the list's one tab stop.
  const body = document.querySelector<HTMLElement>('.folder-body')!;
  ui.fireEvent.scroll(body, { target: { scrollTop: 0 } });
  await ui.waitFor(() =>
    assert.equal(
      table.querySelector('.row')?.getAttribute('aria-rowindex'),
      '2',
    ),
  );
  assert.equal(focused(), last);
  assert.ok(last.isConnected);
  assert.deepEqual(tabStops(table), [last]);
  // Focus returning to the pinned row from outside the list scrolls the
  // list to the row's real place.
  ui.screen.getByRole('button', { name: 'Grid view' }).focus();
  last.focus();
  await ui.waitFor(() =>
    assert.notEqual(
      table.querySelector('.row')?.getAttribute('aria-rowindex'),
      '2',
    ),
  );
  assert.equal(focused(), last);
  ui.fireEvent.keyDown(last, { key: 'PageUp' });
  await ui.waitFor(() =>
    assert.ok(Number(focused().getAttribute('aria-rowindex')) < total + 1),
  );
  ui.fireEvent.keyDown(focused(), { key: 'Home' });
  await ui.waitFor(() =>
    assert.equal(focused().getAttribute('aria-rowindex'), '2'),
  );
  assert.deepEqual(tabStops(table), [focused()]);
});

test('the windowed card grid keeps its focused card mounted in its own line', async () => {
  await mount(crowded(200));
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Grid view' }));
  const table = grid();
  const lines = Number(table.getAttribute('aria-rowcount'));
  const cards = () => [...table.querySelectorAll<HTMLElement>('.row')];
  assert.ok(cards().length < 200, 'the grid is windowed');
  cards()[0].focus();
  ui.fireEvent.keyDown(focused(), { key: 'End' });
  await ui.waitFor(() =>
    assert.equal(
      focused().closest('[role="row"]')?.getAttribute('aria-rowindex'),
      String(lines),
    ),
  );
  const last = focused();
  const body = document.querySelector<HTMLElement>('.folder-body')!;
  ui.fireEvent.scroll(body, { target: { scrollTop: 0 } });
  await ui.waitFor(() =>
    assert.equal(
      table.querySelector('[role="row"]')?.getAttribute('aria-rowindex'),
      '1',
    ),
  );
  assert.equal(focused(), last);
  assert.deepEqual(tabStops(table), [last]);
  assert.equal(
    last.closest('[role="row"]')?.querySelectorAll('.row').length,
    1,
  );
});

test('the card view is a grid of lines that the arrow keys cross in two directions', async () => {
  await mount();
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Grid view' }));
  const table = grid();
  const columns = Number(table.getAttribute('aria-colcount'));
  assert.ok(columns > 1);
  const cards = () => [...table.querySelectorAll<HTMLElement>('.row')];
  assert.equal(
    table.getAttribute('aria-rowcount'),
    String(Math.ceil(cards().length / columns)),
  );
  const lines = ui.within(table).getAllByRole('row');
  assert.equal(lines[0].querySelectorAll('[role="gridcell"]').length, columns);
  assert.equal(cards()[1].getAttribute('aria-colindex'), '2');
  assert.deepEqual(tabStops(table), [cards()[0]]);
  cards()[0].focus();
  ui.fireEvent.keyDown(focused(), { key: 'ArrowRight' });
  assert.equal(focused(), cards()[1]);
  ui.fireEvent.keyDown(focused(), { key: 'ArrowDown' });
  assert.equal(focused(), cards()[1 + columns]);
  assert.equal(focused().closest('[role="row"]'), lines[1]);
  ui.fireEvent.keyDown(focused(), { key: 'ArrowLeft' });
  assert.equal(focused(), cards()[columns]);
  ui.fireEvent.keyDown(focused(), { key: 'ArrowUp' });
  assert.equal(focused(), cards()[0]);
});

test('folder rows join the grid ahead of the items', async () => {
  const { locations } = await mount();
  ui.act(() => locations.setFolder('acct:personal|/'));
  const table = grid();
  const rows = [...table.querySelectorAll<HTMLElement>('.row')];
  assert.ok(rows[0].classList.contains('folder'));
  assert.equal(rows[0].getAttribute('role'), 'row');
  // A folder row has an empty actions cell, so every row has a cell under
  // each column header.
  const headers = ui.within(table).getAllByRole('columnheader').length;
  for (const row of rows)
    assert.equal(row.querySelectorAll('[role="gridcell"]').length, headers);
  assert.deepEqual(tabStops(table), [rows[0]]);
  rows[0].focus();
  ui.fireEvent.keyDown(rows[0], { key: 'ArrowDown' });
  assert.equal(focused(), rows[1]);
  ui.fireEvent.keyDown(focused(), { key: 'ArrowUp' });
  ui.fireEvent.keyDown(focused(), { key: 'Enter' });
  assert.equal(
    locations.getSnapshot().folder,
    `acct:personal|${rows[0].dataset.folderPath}`,
  );
});

test('each folder tree is one tab stop that the arrow keys open, close and walk', async () => {
  const { locations, settings } = await mount();
  const vaults = ui.screen.getByRole('tree', { name: 'Vaults' });
  const teams = ui.screen.getByRole('tree', { name: 'Teams' });
  for (const tree of [vaults, teams]) {
    assert.equal(tabStops(tree).length, 1);
    assert.equal(tree.querySelector('button'), null);
  }
  const items = () => ui.within(vaults).getAllByRole('treeitem');
  const personal = items().find((item) => named(item) === 'Personal')!;
  assert.equal(tabStops(vaults)[0], items()[0]);
  assert.equal(personal.getAttribute('aria-level'), '1');
  assert.equal(personal.getAttribute('aria-expanded'), 'true');
  // Every row drawn above a deeper row, a store with only its Trash below it
  // included, is a parent that states whether it is open.
  for (const tree of [vaults, teams]) {
    const rows = ui.within(tree).getAllByRole('treeitem');
    rows.forEach((row, index) => {
      const next = rows[index + 1];
      if (
        next &&
        Number(next.getAttribute('aria-level')) >
          Number(row.getAttribute('aria-level'))
      )
        assert.equal(row.getAttribute('aria-expanded'), 'true', named(row));
    });
  }
  personal.focus();
  ui.fireEvent.keyDown(personal, { key: 'ArrowRight' });
  const child = focused();
  assert.equal(child.getAttribute('aria-level'), '2');
  assert.deepEqual(tabStops(vaults), [child]);
  ui.fireEvent.keyDown(child, { key: 'ArrowLeft' });
  assert.equal(focused(), personal);
  // Left closes an open store; Right opens it again.
  const drawn = items().length;
  ui.fireEvent.keyDown(personal, { key: 'ArrowLeft' });
  assert.equal(personal.getAttribute('aria-expanded'), 'false');
  assert.ok(items().length < drawn);
  ui.fireEvent.keyDown(personal, { key: 'ArrowRight' });
  assert.equal(personal.getAttribute('aria-expanded'), 'true');
  assert.equal(items().length, drawn);
  // Down walks drawn rows; Enter selects the focused one.
  ui.fireEvent.keyDown(personal, { key: 'ArrowDown' });
  const folder = focused();
  ui.fireEvent.keyDown(folder, { key: 'Enter' });
  assert.equal(
    locations.getSnapshot().folder,
    `acct:personal|${folder.dataset.folderPath}`,
  );
  assert.equal(folder.getAttribute('aria-selected'), 'true');
  // A team's settings, a pointer shortcut on its row, are in its menu too.
  const team = ui.within(teams).getAllByRole('treeitem')[0];
  ui.fireEvent.keyDown(team, { key: 'F10', shiftKey: true });
  ui.fireEvent.click(
    ui.screen.getByRole('menuitem', { name: 'Team settings' }),
  );
  assert.deepEqual(settings, [team.dataset.folderStore]);
});
