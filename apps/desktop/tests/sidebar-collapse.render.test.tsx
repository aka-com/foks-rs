/**
 * The sidebar's hide toggle: the strip's control that hides it, the panel
 * bar's control that shows it again, its stored preference, and the item
 * interactions that must leave the choice alone.
 *
 * Assertions read the DOM and `localStorage` rather than shell state, because
 * both are the contract: hiding is the sidebar's `hidden` attribute and the
 * window's `.app.no-side`, and the preference outlives the process.
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

/** The hide toggle beside the traffic lights. */
function collapseToggle(): HTMLButtonElement {
  const button = document.querySelector<HTMLButtonElement>(
    '.side .traffic .side-collapse',
  );
  assert.ok(button, 'the traffic strip draws the collapse toggle');
  return button;
}

/** The panel bar's control that shows the sidebar again. */
function expandToggle(): HTMLButtonElement {
  const button = ui.screen.getByRole('button', { name: 'Expand sidebar' });
  assert.ok(button instanceof window.HTMLButtonElement);
  assert.ok(button.closest('.topbar'));
  return button;
}

/** Whether the sidebar is hidden. */
function hidden(): boolean {
  return Boolean(document.querySelector('nav.side[hidden]'));
}

test('the toggle hides the sidebar, the panel bar shows it, and the preference is stored', async () => {
  await shell();
  const rows = document.querySelectorAll('.side .nav').length;
  assert.ok(rows > 3);
  assert.equal(hidden(), false);
  assert.equal(collapseToggle().getAttribute('aria-expanded'), 'true');
  assert.equal(collapseToggle().title, 'Collapse sidebar');
  assert.equal(document.querySelector('.topbar .side-expand'), null);

  ui.fireEvent.click(collapseToggle());
  await ui.waitFor(() => assert.ok(hidden()));
  assert.ok(document.querySelector('.app.no-side'));
  assert.equal(expandToggle().getAttribute('aria-expanded'), 'false');
  // The name stays the action; the fixture's waiting requests and notices
  // ride the control as a dot and its description.
  assert.equal(expandToggle().getAttribute('aria-label'), 'Expand sidebar');
  assert.match(expandToggle().title, /^Expand sidebar: .*requests/);
  assert.ok(expandToggle().querySelector('.dot'));
  const described = expandToggle().getAttribute('aria-describedby');
  assert.ok(described);
  assert.match(
    document.getElementById(described)?.textContent ?? '',
    /requests/,
  );
  assert.equal(window.localStorage.getItem('sideCollapsed'), '1');
  // Hiding keeps the sidebar's state: every row is still in the document.
  assert.equal(document.querySelectorAll('.side .nav').length, rows);

  ui.fireEvent.click(expandToggle());
  await ui.waitFor(() => assert.equal(hidden(), false));
  assert.equal(window.localStorage.getItem('sideCollapsed'), '0');
});

test('a hidden sidebar stays hidden under the pointer and the keyboard', async () => {
  await shell();
  ui.fireEvent.click(collapseToggle());
  await ui.waitFor(() => assert.ok(hidden()));
  const field = document.querySelector<HTMLInputElement>('.topbar input');
  assert.ok(field);
  ui.fireEvent.mouseOver(field);
  field.focus();
  assert.ok(hidden());
  assert.ok(document.querySelector('.app.no-side'));
});

test('selecting an item and closing its details leave the sidebar shown', async () => {
  await shell({ kind: 'all' }, '*');
  const stored = window.localStorage.getItem('sideCollapsed');

  const row = document.querySelector<HTMLElement>('.body .row');
  assert.ok(row, 'the items list has a row to select');
  ui.fireEvent.click(row);
  await ui.waitFor(() => {
    assert.ok(document.querySelector('.details:not(.details-empty)'));
  });
  assert.equal(hidden(), false);
  assert.equal(window.localStorage.getItem('sideCollapsed'), stored);

  // An outside click clears the selection and closes the details.
  ui.fireEvent.pointerDown(document.body, { button: 0 });
  await ui.waitFor(() => {
    assert.equal(document.querySelector('.details:not(.details-empty)'), null);
  });
  assert.equal(hidden(), false);
  assert.equal(window.localStorage.getItem('sideCollapsed'), stored);
});

test('an explicit collapse while details are open sticks after closing', async () => {
  await shell({ kind: 'all' }, '*');
  const row = document.querySelector<HTMLElement>('.body .row');
  assert.ok(row);
  ui.fireEvent.click(row);
  await ui.waitFor(() => {
    assert.ok(document.querySelector('.details:not(.details-empty)'));
  });
  assert.equal(hidden(), false);

  ui.fireEvent.click(collapseToggle());
  await ui.waitFor(() => assert.ok(hidden()));
  assert.equal(window.localStorage.getItem('sideCollapsed'), '1');

  // Escape in the details clears the selection and closes them.
  const panel = document.querySelector<HTMLElement>('.details');
  assert.ok(panel);
  ui.fireEvent.keyDown(panel, { key: 'Escape' });
  await ui.waitFor(() => {
    assert.equal(document.querySelector('.details:not(.details-empty)'), null);
  });
  assert.ok(hidden());
  assert.equal(window.localStorage.getItem('sideCollapsed'), '1');
});

test('first run draws no collapse toggle and never hides the sidebar', async () => {
  await shell({ kind: 'first-run', step: 'who' });
  await ui.waitFor(() => {
    assert.ok(document.querySelector('nav.side'));
  });
  assert.equal(document.querySelector('.side-collapse'), null);
  assert.equal(document.querySelector('.app.no-side'), null);
});

test('every place row draws a glyph', async () => {
  await shell();
  const rows = [...document.querySelectorAll('.side .nav[data-place]')];
  assert.equal(rows.length, 8);
  for (const row of rows)
    assert.ok(row.querySelector('.ic'), `${row.textContent} has a glyph`);
});

test('the sidebar resizes by dragging, persists its width, and restores after hiding', async () => {
  await shell();
  const handle = ui.screen.getByRole('separator', { name: 'Resize sidebar' });
  const frame = document.querySelector<HTMLElement>('.window');
  assert.ok(frame);
  assert.equal(handle.getAttribute('aria-valuenow'), '220');
  assert.equal(frame.style.getPropertyValue('--side-w-open'), '220px');
  // jsdom does not implement pointer capture or PointerEvent coordinates.
  handle.setPointerCapture = () => {};
  ui.fireEvent(
    handle,
    new window.MouseEvent('pointerdown', {
      bubbles: true,
      button: 0,
      clientX: 220,
    }),
  );
  ui.fireEvent(
    handle,
    new window.MouseEvent('pointermove', { bubbles: true, clientX: 250 }),
  );
  ui.fireEvent(
    handle,
    new window.MouseEvent('pointerup', { bubbles: true, clientX: 250 }),
  );
  assert.equal(frame.style.getPropertyValue('--side-w-open'), '250px');
  assert.equal(window.localStorage.getItem('sidebarWidth'), '250');
  ui.fireEvent.click(collapseToggle());
  assert.equal(
    ui.screen.queryByRole('separator', { name: 'Resize sidebar' }),
    null,
  );
  ui.fireEvent.click(expandToggle());
  const restored = ui.screen.getByRole('separator', { name: 'Resize sidebar' });
  assert.equal(restored.getAttribute('aria-valuenow'), '250');
  ui.fireEvent.keyDown(restored, { key: 'Home' });
  assert.equal(frame.style.getPropertyValue('--side-w-open'), '200px');
  ui.fireEvent.keyDown(restored, { key: 'ArrowLeft' });
  assert.equal(frame.style.getPropertyValue('--side-w-open'), '200px');
  ui.fireEvent.doubleClick(restored);
  assert.equal(frame.style.getPropertyValue('--side-w-open'), '220px');
  assert.equal(window.localStorage.getItem('sidebarWidth'), '220');
});
