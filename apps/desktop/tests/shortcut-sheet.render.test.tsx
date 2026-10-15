/**
 * The shell's keyboard shortcut sheet and the Lock shortcut: Ctrl / or ?
 * opens the sheet, which lists the registry with this platform's keys; Ctrl L
 * and the macOS menu's Lock lock the app, the menu's Lock whatever the window
 * shows.
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
  // The Chat tab's keys, in this platform's notation; ⌘ forms are not listed.
  assert.ok(
    group(sheet, 'Chat').some(
      ([label, keys]) =>
        label === 'Previous or next channel' && keys === 'Alt ↑ Alt ↓',
    ),
  );
  assert.ok(
    group(sheet, 'Chat composer').some(
      ([label, keys]) => label === 'Bold' && keys === 'Ctrl B',
    ),
  );
  assert.ok(
    group(sheet, 'Chat messages').some(
      ([label, keys]) =>
        label === 'Open the message’s actions menu' &&
        keys === 'Enter Shift F10 Menu',
    ),
  );

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

/**
 * A native bridge whose `onLockMenu` stands in for the macOS menu's Lock:
 * `menuLock()` arms the lock, as the host does, and emits the event.
 */
function menuLockBridge(overrides: Partial<Bridge> = {}) {
  let locked = false;
  let listener: (() => void) | undefined;
  const overridden: Partial<Bridge> = {
    native: true,
    appLockState: async () => ({
      locked,
      available: true,
      mechanism: 'password',
    }),
    onLockMenu: async (next) => {
      listener = next;
      return () => {
        if (listener === next) listener = undefined;
      };
    },
    ...overrides,
  };
  return {
    overrides: overridden,
    listening: () => listener !== undefined,
    menuLock: () => {
      locked = true;
      listener?.();
    },
  };
}

test('the macOS menu’s Lock, which the host arms, shows the lock screen', async () => {
  const menu = menuLockBridge();
  const rendered = await mount(menu.overrides);
  await ui.waitFor(() => assert.ok(menu.listening()));
  ui.act(() => menu.menuLock());
  await rendered.findByRole('button', { name: 'Unlock' });
});

async function bootWithoutSnapshot(overrides: Partial<Bridge>) {
  const { App } = (await vite.ssrLoadModule(
    '/src/app-root.tsx',
  )) as typeof import('../src/app-root');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  return ui.render(
    createElement(App, { bridge: { ...mockBridge(FIXTURE), ...overrides } }),
  );
}

test('the macOS menu’s Lock shows the lock screen over the loading screen', async () => {
  // The agent never reports, so the window stays on the loading screen.
  const menu = menuLockBridge({ agentStatus: () => new Promise(() => {}) });
  const rendered = await bootWithoutSnapshot(menu.overrides);
  await rendered.findByRole('status');
  await ui.waitFor(() => assert.ok(menu.listening()));
  ui.act(() => menu.menuLock());
  await rendered.findByRole('dialog', { name: 'FOKS is locked' });
});

test('the macOS menu’s Lock shows the lock screen over a boot error', async () => {
  const menu = menuLockBridge({
    agentStatus: async () => {
      throw new Error('The agent status failed.');
    },
  });
  const rendered = await bootWithoutSnapshot(menu.overrides);
  await rendered.findByRole('alertdialog', { name: 'FOKS could not start' });
  await ui.waitFor(() => assert.ok(menu.listening()));
  ui.act(() => menu.menuLock());
  await rendered.findByRole('dialog', { name: 'FOKS is locked' });
  assert.equal(
    rendered.queryByRole('alertdialog', { name: 'FOKS could not start' }),
    null,
  );
});
