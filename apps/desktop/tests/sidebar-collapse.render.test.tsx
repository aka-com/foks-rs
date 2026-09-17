/**
 * The collapsible sidebar: the manual toggle and its stored preference, the
 * blur-on-mouse rule, and the layout reaction to the details panel.
 *
 * Assertions read the DOM and `localStorage` rather than shell state, because
 * both are the contract: collapsing is CSS driven by `.side.is-narrow`, and the
 * preference outlives the process.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import { installDom } from './lib/dom-harness';

const dom = installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
});

let vite: ViteDevServer;
let ui: typeof import('@testing-library/react');

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
  window.history.replaceState(null, '', '/');
});

test.after(async () => {
  await vite.close();
  dom.window.close();
});

/**
 * Boots the shell at `location`, with the stored preference already cleared.
 * `view` pins the item view where a test needs to click an item rather than a
 * folder; item pages open in the folder browser by default.
 */
async function shell(
  location?: { kind: 'first-run'; step: 'who' } | { kind: 'all' },
  view?: 'list' | 'grid' | 'folders',
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
  window.localStorage.clear();
  const locations = new LocationStore({
    ...INITIAL_STATE,
    location: location ?? { kind: 'all' },
    ...(view ? { view } : {}),
  });
  const rendered = ui.render(
    createElement(App, {
      snapshot: FIXTURE,
      bridge: mockBridge(FIXTURE),
      store: locations,
    }),
  );
  await ui.waitFor(() => assert.ok(document.querySelector('.app')));
  return rendered;
}

/** The sidebar's collapse toggle. */
function toggle(): HTMLButtonElement {
  const button = document.querySelector<HTMLButtonElement>('.side-collapse');
  assert.ok(button, 'the footer draws the collapse toggle');
  return button;
}

test('the toggle collapses the rail, flips its label, and stores the preference', async () => {
  await shell();
  const rows = document.querySelectorAll('.side .nav').length;
  assert.ok(rows > 3);
  assert.equal(
    document.querySelector('.side')?.classList.contains('is-narrow'),
    false,
  );
  assert.equal(toggle().getAttribute('aria-expanded'), 'true');

  ui.fireEvent.click(toggle());
  await ui.waitFor(() => {
    assert.ok(document.querySelector('.side.is-narrow'));
  });
  assert.ok(document.querySelector('.app.side-narrow'));
  assert.equal(toggle().getAttribute('aria-expanded'), 'false');
  assert.equal(toggle().title, 'Expand');
  assert.equal(window.localStorage.getItem('sideCollapsed'), '1');
  // Collapsing is CSS: every row is still in the document.
  assert.equal(document.querySelectorAll('.side .nav').length, rows);

  ui.fireEvent.click(toggle());
  await ui.waitFor(() => {
    assert.equal(document.querySelector('.side.is-narrow'), null);
  });
  assert.equal(window.localStorage.getItem('sideCollapsed'), '0');
  assert.equal(window.localStorage.getItem('sidePinned'), '1');
});

test('a width chosen by hand pins the rail, and a mouse activation drops focus', async () => {
  await shell();
  assert.equal(
    document.querySelector('.side')?.classList.contains('is-pinned'),
    false,
  );

  toggle().focus();
  ui.fireEvent.click(toggle(), { detail: 1 });
  await ui.waitFor(() => {
    assert.ok(document.querySelector('.side.is-pinned'));
  });
  assert.equal(window.localStorage.getItem('sidePinned'), '1');
  // A mouse click must not hold the rail open through `:focus-within`.
  assert.notEqual(document.activeElement, toggle());

  toggle().focus();
  ui.fireEvent.click(toggle(), { detail: 0 });
  await ui.waitFor(() => {
    assert.equal(document.querySelector('.side.is-narrow'), null);
  });
  // Keyboard activation keeps focus, so the rail stays reachable.
  assert.equal(document.activeElement, toggle());
});

test('a row activated with the mouse drops focus, and with the keyboard keeps it', async () => {
  await shell();
  const row = [
    ...document.querySelectorAll<HTMLButtonElement>('.side .nav'),
  ].find((candidate) => candidate.textContent?.startsWith('People'));
  assert.ok(row);
  row.focus();
  ui.fireEvent.click(row, { detail: 1 });
  assert.notEqual(document.activeElement, row);
  row.focus();
  ui.fireEvent.click(row, { detail: 0 });
  assert.equal(document.activeElement, row);
});

test('opening details collapses the rail and closing it restores the chosen width', async () => {
  await shell({ kind: 'all' }, 'list');
  const stored = window.localStorage.getItem('sideCollapsed');

  const row = document.querySelector<HTMLElement>('.body .row');
  assert.ok(row, 'the items list has a row to select');
  ui.fireEvent.click(row);
  await ui.waitFor(() => {
    assert.ok(document.querySelector('.details'));
    assert.ok(document.querySelector('.side.is-narrow'));
  });
  // A layout reaction, not a preference.
  assert.equal(window.localStorage.getItem('sideCollapsed'), stored);

  const close = document.querySelector<HTMLButtonElement>(
    '.details [aria-label="Close"]',
  );
  assert.ok(close);
  ui.fireEvent.click(close);
  await ui.waitFor(() => {
    assert.equal(document.querySelector('.details'), null);
    assert.equal(document.querySelector('.side.is-narrow'), null);
  });
  assert.equal(window.localStorage.getItem('sideCollapsed'), stored);
});

test('an expand made while details are open sticks', async () => {
  await shell({ kind: 'all' }, 'list');
  const row = document.querySelector<HTMLElement>('.body .row');
  assert.ok(row);
  ui.fireEvent.click(row);
  await ui.waitFor(() => {
    assert.ok(document.querySelector('.side.is-narrow'));
  });

  ui.fireEvent.click(toggle());
  await ui.waitFor(() => {
    assert.equal(document.querySelector('.side.is-narrow'), null);
  });
  // The panel is still open; nothing re-collapses the rail behind the reader.
  assert.ok(document.querySelector('.details'));
  assert.equal(window.localStorage.getItem('sideCollapsed'), '0');
});

test('first run draws no collapse toggle and never narrows the shell', async () => {
  await shell({ kind: 'first-run', step: 'who' });
  await ui.waitFor(() => {
    assert.ok(document.querySelector('nav.side'));
  });
  assert.equal(document.querySelector('.side-collapse'), null);
  assert.equal(document.querySelector('.app.side-narrow'), null);
});

test('every tab draws a glyph, so all six survive the collapsed rail', async () => {
  await shell();
  const tabs = [...document.querySelectorAll('.side.rail .rail-tabs .nav')];
  assert.deepEqual(
    tabs.map((tab) => tab.querySelector('.t')?.textContent),
    ['People', 'Chat', 'Files', 'Teams', 'Devices', 'Settings'],
  );
  for (const tab of tabs)
    assert.ok(tab.querySelector('.ic'), `${tab.textContent} has a glyph`);
});
