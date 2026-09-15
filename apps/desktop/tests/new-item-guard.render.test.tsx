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
  return { rendered, store };
}

/** Opens the sheet for `kind` from the toolbar's New menu. */
async function openSheet(
  rendered: Awaited<ReturnType<typeof mount>>['rendered'],
  kind: string,
): Promise<void> {
  ui.fireEvent.click(rendered.getByRole('button', { name: 'New' }));
  ui.fireEvent.click(await rendered.findByRole('menuitem', { name: kind }));
  await ui.waitFor(() =>
    assert.ok(rendered.getByText(`New ${kind.toLowerCase()}`)),
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
  const found = [...panel.querySelectorAll<HTMLButtonElement>('.ft .btn')];
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
    assert.equal(rendered.queryByText('New password'), null),
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
  assert.equal(panel.querySelector('.hd h2')?.textContent, 'Discard new item?');
  assert.equal(
    panel.querySelector('.sb p')?.textContent,
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
    assert.equal(rendered.queryByText('New password'), null),
  );
});

test('cancelling the discard keeps the draft and the reader in place', async () => {
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
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Create item' }));
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
  await openSheet(rendered, 'Link');
  ui.fireEvent.change(rendered.getByLabelText('Target path'), {
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
  assert.equal(panel.querySelector('.hd h2')?.textContent, 'Discard new item?');
  assert.equal(
    panel.querySelector('.sb p')?.textContent,
    'Your new link has not been saved.',
  );
  // The draft survives the question until it is answered.
  await ui.act(async () => {
    ui.fireEvent.click(dialogButton('Cancel'));
    await Promise.resolve();
  });
  assert.equal(
    (rendered.getByLabelText('Target path') as HTMLInputElement).value,
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
  await ui.waitFor(() => assert.equal(rendered.queryByText('New link'), null));
});
