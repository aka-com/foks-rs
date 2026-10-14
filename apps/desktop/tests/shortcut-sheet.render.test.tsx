/**
 * The shell's keyboard shortcut sheet and the Lock shortcut: Ctrl / or ?
 * opens the sheet, which lists the registry with this platform's keys; Ctrl L
 * and the macOS menu's Lock lock the app.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import type { Bridge } from '../src/bridge';
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
  const rendered = ui.render(
    createElement(App, {
      snapshot: FIXTURE,
      bridge: { ...mockBridge(FIXTURE), ...overrides },
      store,
    }),
  );
  await ui.waitFor(() => assert.ok(rendered.getAllByText('Personal').length));
  return rendered;
}

/** The sheet's rows in the group captioned `caption`, as [label, keys]. */
function group(sheet: HTMLElement, caption: string): string[][] {
  const table = ui.within(sheet).getByRole('table', { name: caption });
  return [...table.querySelectorAll('tr')].map((row) => [
    row.querySelector('th')?.textContent ?? '',
    [...row.querySelectorAll('kbd')].map((key) => key.textContent).join(' '),
  ]);
}

test('Ctrl / opens the shortcut sheet, which lists this platform’s keys', async () => {
  const rendered = await mount();
  assert.equal(
    ui.fireEvent.keyDown(document, { key: '/', ctrlKey: true }),
    false,
  );
  const sheet = await rendered.findByRole('dialog', {
    name: 'Keyboard shortcuts',
  });
  const anywhere = group(sheet, 'Anywhere');
  assert.ok(
    anywhere.some(([label, keys]) => label === 'Lock' && keys === 'Ctrl L'),
  );
  assert.ok(
    anywhere.some(([label, keys]) => label === 'Back' && keys === 'Alt ←'),
  );
  // Settings has a shortcut only in the macOS menu.
  assert.ok(!anywhere.some(([label]) => label === 'Settings'));
  assert.ok(
    group(sheet, 'Files list').some(
      ([label, keys]) => label === 'Rename or move' && keys === 'F2',
    ),
  );
  assert.ok(group(sheet, 'Folder tree').length > 0);
  assert.ok(group(sheet, 'Search palette').length > 0);

  ui.fireEvent.keyDown(document.activeElement ?? sheet, { key: 'Escape' });
  await ui.waitFor(() =>
    assert.equal(
      rendered.queryByRole('dialog', { name: 'Keyboard shortcuts' }),
      null,
    ),
  );
});

test('? opens the sheet outside a text field and is typed inside one', async () => {
  const rendered = await mount();
  const field = document.querySelector<HTMLInputElement>('.app input');
  assert.ok(field, 'the page has a text field');
  ui.fireEvent.keyDown(field, { key: '?', shiftKey: true });
  assert.equal(
    rendered.queryByRole('dialog', { name: 'Keyboard shortcuts' }),
    null,
  );
  ui.fireEvent.keyDown(document.body, { key: '?', shiftKey: true });
  await rendered.findByRole('dialog', { name: 'Keyboard shortcuts' });
  // A second press does not open another sheet over the first.
  ui.fireEvent.keyDown(document.body, { key: '/', ctrlKey: true });
  assert.equal(
    rendered.getAllByRole('dialog', { name: 'Keyboard shortcuts' }).length,
    1,
  );
});

test('the search palette’s Keyboard shortcuts action opens the sheet', async () => {
  const rendered = await mount();
  ui.fireEvent.keyDown(document, { key: 'k', ctrlKey: true });
  const palette = await rendered.findByRole('dialog', { name: /Search/ });
  const action = ui
    .within(palette)
    .getByText('Keyboard shortcuts')
    .closest<HTMLElement>('[role="option"]');
  assert.ok(action);
  assert.equal(action.querySelector('kbd')?.textContent, 'Ctrl /');
  ui.fireEvent.click(action);
  await rendered.findByRole('dialog', { name: 'Keyboard shortcuts' });
});

test('Ctrl L locks the app from any page', async () => {
  let locks = 0;
  const rendered = await mount({
    lockApp: async () => {
      locks += 1;
      return { locked: true, available: true, mechanism: 'password' };
    },
  });
  assert.equal(
    ui.fireEvent.keyDown(document, { key: 'l', ctrlKey: true }),
    false,
  );
  await ui.waitFor(() => assert.equal(locks, 1));
  await rendered.findByRole('button', { name: 'Unlock' });
});

test('the macOS menu’s Lock locks the app through the same path', async () => {
  let locks = 0;
  let menuLock: (() => void) | undefined;
  const rendered = await mount({
    native: true,
    lockApp: async () => {
      locks += 1;
      return { locked: true, available: true, mechanism: 'password' };
    },
    onLockMenu: async (listener) => {
      menuLock = listener;
      return () => {};
    },
  });
  await ui.waitFor(() => assert.ok(menuLock));
  ui.act(() => menuLock?.());
  await ui.waitFor(() => assert.equal(locks, 1));
  await rendered.findByRole('button', { name: 'Unlock' });
});
