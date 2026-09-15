/**
 * The ⌘K palette as a sheet: the shortcut that opens it, the keyboard walk
 * over the grouped rows, what Enter navigates to, the scope chips Tab cycles,
 * the note on what search does not cover, and Escape.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, useState } from 'react';
import type { ReactNode } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import { installDom } from './lib/dom-harness';
import type { Location } from '../src/location';
import type { StoreRef } from '../src/model';
import type { SearchChannel } from '../src/shell/search-palette';

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

const CHANNELS: readonly SearchChannel[] = [
  { store: 'team:eng', name: 'deploys', id: 'a'.repeat(32) },
  { store: 'team:household', name: 'general', id: 'b'.repeat(32) },
];

interface Journal {
  navigations: Location[];
  items: { store: StoreRef; path: string }[];
  closes: number;
}

/**
 * Mounts the palette behind the shortcut, exactly as the shell will: a host
 * that owns the `open` flag and hands the palette its `onClose`.
 */
async function palette(
  open = false,
  /** Mount another modal dialog beside the palette, as a sheet would be. */
  dialog = false,
) {
  const { SearchPalette, useSearchShortcut } = (await vite.ssrLoadModule(
    '/src/shell/search-palette.tsx',
  )) as typeof import('../src/shell/search-palette');
  const { Dialog, OverlayProvider } = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');

  const journal: Journal = { navigations: [], items: [], closes: 0 };
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);

  function Host(): ReactNode {
    const [shown, setShown] = useState(open);
    useSearchShortcut(() => {
      setShown(true);
    });
    return createElement(SearchPalette, {
      snapshot: FIXTURE,
      open: shown,
      channels: CHANNELS,
      onClose: () => {
        journal.closes += 1;
        setShown(false);
      },
      onNavigate: (next: Location) => journal.navigations.push(next),
      onOpenItem: (store: StoreRef, path: string) => {
        journal.items.push({ store, path });
      },
    });
  }

  const rendered = ui.render(
    createElement(OverlayProvider, {
      backgroundRef: { current: null },
      portalRoot,
      children: [
        createElement(Host, { key: 'host' }),
        dialog
          ? createElement(Dialog, { key: 'dialog', children: 'A sheet' })
          : null,
      ],
    }),
  );
  return { rendered, journal };
}

function field(): HTMLInputElement {
  const input = document.querySelector<HTMLInputElement>('.pal-q input');
  assert.ok(input, 'the palette draws its query field');
  return input;
}

function rows(): HTMLElement[] {
  return [...document.querySelectorAll<HTMLElement>('.pal-hit')];
}

function headings(): string[] {
  return [...document.querySelectorAll('.pal-grp')].map(
    (node) => node.textContent ?? '',
  );
}

/** The row the field's `aria-activedescendant` points at. */
function activeRow(): HTMLElement {
  const id = field().getAttribute('aria-activedescendant');
  assert.ok(id, 'the field names an active row');
  const row = document.getElementById(id);
  assert.ok(row, 'the active row is in the list');
  return row;
}

function type(value: string): void {
  ui.fireEvent.change(field(), { target: { value } });
}

test('the palette draws nothing until ⌘K opens it', async () => {
  await palette();
  assert.equal(document.querySelector('.pal'), null);

  ui.fireEvent.keyDown(document, { key: 'k', metaKey: true });
  await ui.waitFor(() => {
    assert.ok(document.querySelector('.pal'), 'the sheet is up');
  });
  // The field takes focus, so the next keystroke is the query.
  assert.equal(document.activeElement, field());
  // Ctrl-K is the same shortcut away from a Mac; it reopens without complaint.
  ui.fireEvent.keyDown(document, { key: 'K', ctrlKey: true });
  assert.ok(document.querySelector('.pal'));
});

test('⌘K does not open the palette while another modal is active', async () => {
  await palette(false, true);
  assert.ok(document.querySelector('[role="dialog"]'), 'the sheet is up');
  ui.fireEvent.keyDown(document, { key: 'k', metaKey: true });
  // The document-level shortcut checks modal state because the active dialog
  // cannot intercept the listener before it runs.
  assert.equal(document.querySelector('.pal'), null);
});

test('results are grouped by kind and counted for a screen reader', async () => {
  await palette(true);
  assert.deepEqual(headings(), [
    'Items',
    'Vaults, groups and shares',
    'People',
    'Channels',
  ]);

  type('prod');
  // Only the items whose name or path carries `prod`, and nothing else.
  assert.deepEqual(headings(), ['Items']);
  assert.deepEqual(
    rows().map((row) => row.querySelector('.t b')?.textContent),
    ['production-token', 'DATABASE_URL'],
  );
  // The matched run is marked inside the name.
  assert.equal(rows()[0].querySelector('.t b em')?.textContent, 'prod');
  assert.equal(
    rows()[1].querySelector('.t small')?.textContent,
    'Note · env/prod · Personal',
  );

  const count = document.querySelector('.pal-count');
  assert.ok(count);
  assert.equal(count.getAttribute('aria-live'), 'polite');
  assert.equal(count.textContent, '2 results');

  type('netflix');
  assert.equal(document.querySelector('.pal-count')?.textContent, '1 result');
});

test('arrow keys move selection across result groups', async () => {
  await palette(true);
  type('e');
  const all = rows();
  assert.ok(all.length > 6, 'the query reaches more than one group');
  assert.equal(activeRow(), all[0]);
  assert.equal(all[0].getAttribute('aria-selected'), 'true');

  // Down walks to the end of the Items group and straight into the next one.
  const items = all.filter(
    (row) =>
      row.closest('[role="group"]')?.getAttribute('aria-label') === 'Items',
  );
  for (let step = 0; step < items.length; step += 1)
    ui.fireEvent.keyDown(field(), { key: 'ArrowDown' });
  assert.equal(
    activeRow().closest('[role="group"]')?.getAttribute('aria-label'),
    'Vaults, groups and shares',
  );
  ui.fireEvent.keyDown(field(), { key: 'ArrowUp' });
  assert.equal(activeRow(), items[items.length - 1]);

  // Neither end runs off the list.
  for (let step = 0; step < 50; step += 1)
    ui.fireEvent.keyDown(field(), { key: 'ArrowUp' });
  assert.equal(activeRow(), rows()[0]);
  for (let step = 0; step < 200; step += 1)
    ui.fireEvent.keyDown(field(), { key: 'ArrowDown' });
  assert.equal(activeRow(), rows()[rows().length - 1]);
  // The field never loses focus to the rows it is walking.
  assert.equal(document.activeElement, field());
});

test('Enter opens the highlighted row and closes the palette', async () => {
  const { journal } = await palette(true);
  type('production-token');
  ui.fireEvent.keyDown(field(), { key: 'Enter' });
  // Send the store and item selection through one callback so the host can apply
  // them atomically after navigation-guard evaluation.
  assert.deepEqual(journal.items.at(-1), {
    store: 'team:eng',
    path: '/deploy/production-token',
  });
  assert.deepEqual(journal.navigations, []);
  assert.equal(journal.closes, 1);
  assert.equal(document.querySelector('.pal'), null);
});

test('each kind of result goes to the tab that owns it', async () => {
  const { journal } = await palette(true);

  const openFirst = (query: string, scope: string): void => {
    type(query);
    const chip = [...document.querySelectorAll('.pal-scopes .chip')].find(
      (button) => button.textContent === scope,
    );
    assert.ok(chip, `the ${scope} chip is drawn`);
    ui.fireEvent.click(chip);
    ui.fireEvent.click(rows()[0]);
  };

  openFirst('Household', 'Vaults & groups');
  assert.deepEqual(journal.navigations.at(-1), {
    kind: 'store',
    ref: 'team:household',
  });

  ui.fireEvent.keyDown(document, { key: 'k', metaKey: true });
  openFirst('priya', 'People');
  // A person is a row of a group's Members tab.
  assert.deepEqual(journal.navigations.at(-1), {
    kind: 'group-settings',
    ref: 'team:eng',
    tab: 'people',
  });

  ui.fireEvent.keyDown(document, { key: 'k', metaKey: true });
  openFirst('deploys', 'Channels');
  assert.deepEqual(journal.navigations.at(-1), {
    kind: 'chat',
    ref: 'team:eng',
    channel: 'a'.repeat(32),
  });
  assert.equal(journal.closes, 3);
});

test('Tab cycles the scope chips and never moves focus', async () => {
  await palette(true);
  const labels = (): string[] =>
    [...document.querySelectorAll('.pal-scopes .chip')].map(
      (chip) => chip.textContent ?? '',
    );
  const current = (): string | undefined =>
    [...document.querySelectorAll('.pal-scopes .chip')].find((chip) =>
      chip.className.includes('on'),
    )?.textContent ?? undefined;

  assert.deepEqual(labels(), [
    'All',
    'Items',
    'Vaults & groups',
    'People',
    'Channels',
  ]);
  assert.equal(current(), 'All');

  ui.fireEvent.keyDown(field(), { key: 'Tab' });
  assert.equal(current(), 'Items');
  assert.deepEqual(headings(), ['Items']);
  assert.equal(document.activeElement, field());

  // Shift-Tab walks back, and both ends wrap.
  ui.fireEvent.keyDown(field(), { key: 'Tab', shiftKey: true });
  assert.equal(current(), 'All');
  ui.fireEvent.keyDown(field(), { key: 'Tab', shiftKey: true });
  assert.equal(current(), 'Channels');
  assert.deepEqual(headings(), ['Channels']);
  ui.fireEvent.keyDown(field(), { key: 'Tab' });
  assert.equal(current(), 'All');
});

test('an empty result says what search does not cover', async () => {
  await palette(true);
  type('zzzznothing');
  assert.equal(rows().length, 0);
  const none = document.querySelector('.pal-none');
  assert.ok(none);
  assert.match(none.textContent ?? '', /No matches for “zzzznothing”/);
  assert.match(none.textContent ?? '', /not item contents/);
  assert.equal(document.querySelector('.pal-count')?.textContent, '0 results');
  // With nothing highlighted, Enter does nothing rather than opening a row.
  ui.fireEvent.keyDown(field(), { key: 'Enter' });
  assert.ok(document.querySelector('.pal'), 'the palette is still open');
});

test('Escape closes the palette, and it reopens empty', async () => {
  const { journal } = await palette(true);
  type('netflix');
  ui.fireEvent.keyDown(field(), { key: 'Escape' });
  assert.equal(journal.closes, 1);
  assert.equal(document.querySelector('.pal'), null);
  assert.deepEqual(journal.navigations, []);

  ui.fireEvent.keyDown(document, { key: 'k', metaKey: true });
  await ui.waitFor(() => {
    assert.ok(document.querySelector('.pal'));
  });
  // The palette is a sheet, not a place: nothing about the last one survives.
  assert.equal(field().value, '');
  assert.deepEqual(headings(), [
    'Items',
    'Vaults, groups and shares',
    'People',
    'Channels',
  ]);
});
