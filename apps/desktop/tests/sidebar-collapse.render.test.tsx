/**
 * The collapsible rail: the topbar's toggle, its stored preference, and the
 * item interactions that must leave the chosen width alone.
 *
 * Assertions read the DOM and `localStorage` rather than shell state, because
 * both are the contract: collapsing is CSS driven by `.side.is-narrow`, and the
 * preference outlives the process. The rail no longer expands on hover or on
 * focus, so the width in the DOM is the whole of the behavior.
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
 * `folder` opens the flat "All items" view when a test needs a
 * clickable row rather than the plain store picker.
 */
async function shell(
  location?: { kind: 'first-run'; step: 'who' } | { kind: 'all' },
  folder?: string,
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
    ...(folder ? { folder } : {}),
  });
  const rendered = ui.render(
    createElement(App, {
      snapshot: FIXTURE,
      bridge: mockBridge(FIXTURE),
      store: locations,
    }),
  );
  await ui.waitFor(() => assert.ok(document.querySelector('.app')));
  await ui.waitFor(() => assert.ok(document.querySelector('nav.side')));
  return rendered;
}

/** The collapse toggle beside the traffic lights. */
function collapseToggle(): HTMLButtonElement {
  const button = document.querySelector<HTMLButtonElement>(
    '.side .traffic .side-collapse',
  );
  assert.ok(button, 'the traffic strip draws the collapse toggle');
  return button;
}

function expandToggle(): HTMLButtonElement {
  const button = ui.screen.getByRole('button', { name: 'Expand sidebar' });
  assert.ok(button instanceof window.HTMLButtonElement);
  assert.ok(button.classList.contains('rail-brand'));
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
  assert.equal(collapseToggle().getAttribute('aria-expanded'), 'true');
  assert.equal(collapseToggle().title, 'Collapse sidebar');
  assert.equal(document.querySelector('.topbar .side-collapse'), null);
  const mark = document.querySelector('.rail-brand .mark');
  assert.ok(mark);
  assert.equal(mark.textContent?.trim(), '');
  assert.ok(mark.querySelector('svg.ic'));

  ui.fireEvent.click(collapseToggle());
  await ui.waitFor(() => {
    assert.ok(document.querySelector('.side.is-narrow'));
  });
  assert.ok(document.querySelector('.app.side-narrow'));
  assert.equal(document.querySelector('.side-collapse'), null);
  assert.equal(expandToggle().getAttribute('aria-expanded'), 'false');
  assert.equal(expandToggle().title, 'Expand sidebar');
  assert.equal(window.localStorage.getItem('sideCollapsed'), '1');
  // Collapsing is CSS: every row is still in the document.
  assert.equal(document.querySelectorAll('.side .nav').length, rows);

  ui.fireEvent.click(expandToggle());
  await ui.waitFor(() => {
    assert.equal(document.querySelector('.side.is-narrow'), null);
  });
  assert.equal(window.localStorage.getItem('sideCollapsed'), '0');
});

test('a collapsed rail stays collapsed under the pointer and the keyboard', async () => {
  await shell();
  ui.fireEvent.click(collapseToggle());
  await ui.waitFor(() => {
    assert.ok(document.querySelector('.side.is-narrow'));
  });
  const rail = document.querySelector<HTMLElement>('.side.rail');
  assert.ok(rail);
  const tab = rail.querySelector<HTMLButtonElement>('.rail-tabs .nav');
  assert.ok(tab);
  // Nothing widens the rail but the toggle: the hover and focus expansions the
  // store tree needed are gone.
  ui.fireEvent.mouseOver(rail);
  tab.focus();
  assert.equal(document.activeElement, tab);
  assert.ok(document.querySelector('.side.is-narrow'));
  assert.ok(document.querySelector('.app.side-narrow'));
});

test('selecting an item and closing its details leave the rail expanded', async () => {
  await shell({ kind: 'all' }, '*');
  const stored = window.localStorage.getItem('sideCollapsed');

  const row = document.querySelector<HTMLElement>('.body .row');
  assert.ok(row, 'the items list has a row to select');
  ui.fireEvent.click(row);
  await ui.waitFor(() => {
    assert.ok(document.querySelector('.details'));
  });
  assert.equal(document.querySelector('.side.is-narrow'), null);
  assert.equal(window.localStorage.getItem('sideCollapsed'), stored);

  const close = document.querySelector<HTMLButtonElement>(
    '.details [aria-label="Close"]',
  );
  assert.ok(close);
  ui.fireEvent.click(close);
  await ui.waitFor(() => {
    assert.equal(document.querySelector('.details'), null);
  });
  assert.equal(document.querySelector('.side.is-narrow'), null);
  assert.equal(window.localStorage.getItem('sideCollapsed'), stored);
});

test('an explicit collapse while details are open sticks after closing', async () => {
  await shell({ kind: 'all' }, '*');
  const row = document.querySelector<HTMLElement>('.body .row');
  assert.ok(row);
  ui.fireEvent.click(row);
  await ui.waitFor(() => {
    assert.ok(document.querySelector('.details'));
  });
  assert.equal(document.querySelector('.side.is-narrow'), null);

  ui.fireEvent.click(collapseToggle());
  await ui.waitFor(() => {
    assert.ok(document.querySelector('.side.is-narrow'));
  });
  assert.equal(window.localStorage.getItem('sideCollapsed'), '1');

  const close = document.querySelector<HTMLButtonElement>(
    '.details [aria-label="Close"]',
  );
  assert.ok(close);
  ui.fireEvent.click(close);
  await ui.waitFor(() => {
    assert.equal(document.querySelector('.details'), null);
  });
  assert.ok(document.querySelector('.side.is-narrow'));
  assert.equal(window.localStorage.getItem('sideCollapsed'), '1');
});

test('first run draws no collapse toggle and never narrows the shell', async () => {
  await shell({ kind: 'first-run', step: 'who' });
  await ui.waitFor(() => {
    assert.ok(document.querySelector('nav.side'));
  });
  assert.equal(document.querySelector('.side-collapse'), null);
  assert.equal(document.querySelector('.app.side-narrow'), null);
});

test('every tab draws a glyph, so all five survive the collapsed rail', async () => {
  await shell();
  const tabs = [...document.querySelectorAll('.side.rail .rail-tabs .nav')];
  assert.deepEqual(
    tabs.map((tab) => tab.querySelector('.t')?.textContent),
    ['Files', 'Chat', 'Teams', 'Devices', 'Settings'],
  );
  for (const tab of tabs)
    assert.ok(tab.querySelector('.ic'), `${tab.textContent} has a glyph`);
});

test('the sidebar resizes by dragging, persists its width, and restores after collapse', async () => {
  await shell();
  const handle = ui.screen.getByRole('separator', { name: 'Resize sidebar' });
  const frame = document.querySelector<HTMLElement>('.window');
  assert.ok(frame);
  assert.equal(handle.getAttribute('aria-valuenow'), '150');
  assert.equal(frame.style.getPropertyValue('--side-w-open'), '150px');
  // jsdom does not implement pointer capture or PointerEvent coordinates.
  handle.setPointerCapture = () => {};
  ui.fireEvent(
    handle,
    new window.MouseEvent('pointerdown', {
      bubbles: true,
      button: 0,
      clientX: 150,
    }),
  );
  ui.fireEvent(
    handle,
    new window.MouseEvent('pointermove', { bubbles: true, clientX: 230 }),
  );
  ui.fireEvent(
    handle,
    new window.MouseEvent('pointerup', { bubbles: true, clientX: 230 }),
  );
  assert.equal(frame.style.getPropertyValue('--side-w-open'), '230px');
  assert.equal(window.localStorage.getItem('sidebarWidth'), '230');
  ui.fireEvent.click(collapseToggle());
  assert.equal(
    ui.screen.queryByRole('separator', { name: 'Resize sidebar' }),
    null,
  );
  ui.fireEvent.click(expandToggle());
  const restored = ui.screen.getByRole('separator', { name: 'Resize sidebar' });
  assert.equal(restored.getAttribute('aria-valuenow'), '230');
  ui.fireEvent.keyDown(restored, { key: 'Home' });
  assert.equal(frame.style.getPropertyValue('--side-w-open'), '150px');
  ui.fireEvent.keyDown(restored, { key: 'ArrowLeft' });
  assert.equal(frame.style.getPropertyValue('--side-w-open'), '150px');
  ui.fireEvent.doubleClick(restored);
  assert.equal(frame.style.getPropertyValue('--side-w-open'), '150px');
  assert.equal(window.localStorage.getItem('sidebarWidth'), '150');
});
