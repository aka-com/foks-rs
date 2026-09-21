import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';
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
  window.localStorage.clear();
});
test.after(async () => vite.close());

async function mount() {
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
    location: { kind: 'files' },
    folder: 'acct:personal|/documents',
  });
  const view = ui.render(
    createElement(App, {
      store,
      snapshot: FIXTURE,
      bridge: mockBridge(FIXTURE),
    }),
  );
  await ui.waitFor(() =>
    assert.ok(view.getByRole('navigation', { name: 'Breadcrumb' })),
  );
  return { store, view };
}

async function swipe(deltaX: number) {
  await ui.act(async () => {
    window.dispatchEvent(new window.WheelEvent('wheel', { deltaX }));
    await new Promise((resolve) => setTimeout(resolve, 160));
  });
}

test('the header omits history controls', async () => {
  const { store, view } = await mount();
  assert.equal(view.queryByRole('button', { name: 'Back' }), null);
  assert.equal(view.queryByRole('button', { name: 'Forward' }), null);
  assert.equal(view.queryByRole('button', { name: 'Parent folder' }), null);
  assert.equal(view.queryByRole('button', { name: 'Back to All items' }), null);
  ui.act(() => store.navigate({ kind: 'settings', section: 'preferences' }));
  assert.equal(view.queryByRole('button', { name: 'Back' }), null);
  assert.equal(view.queryByRole('button', { name: 'Forward' }), null);
});

test('completed swipes use guarded history in both directions', async () => {
  const { store, view } = await mount();
  ui.act(() => store.navigate({ kind: 'settings', section: 'preferences' }));
  store.registerGuard(() => ({
    verdict: 'prompt',
    title: 'Discard history test draft?',
    body: 'Unsaved changes',
    confirm: 'Discard changes',
  }));
  await swipe(-120);
  assert.ok(view.getByRole('alertdialog'));
  assert.equal(store.getSnapshot().location.kind, 'settings');
  ui.fireEvent.click(view.getByRole('button', { name: 'Cancel' }));
  assert.equal(store.getSnapshot().location.kind, 'settings');
  await swipe(-120);
  await ui.act(async () =>
    ui.fireEvent.click(view.getByRole('button', { name: 'Discard changes' })),
  );
  assert.equal(store.getSnapshot().location.kind, 'files');
  await swipe(120);
  assert.ok(view.getByRole('alertdialog'));
  await ui.act(async () =>
    ui.fireEvent.click(view.getByRole('button', { name: 'Discard changes' })),
  );
  assert.equal(store.getSnapshot().location.kind, 'settings');
});

test('swipe refusal is visible and navigation by another input cancels its pending gesture', async () => {
  const { store, view } = await mount();
  ui.act(() => store.navigate({ kind: 'settings', section: 'preferences' }));
  const remove = store.registerGuard(() => ({
    verdict: 'refuse',
    reason: 'History test save in progress',
  }));
  await swipe(-120);
  assert.ok(view.getByText('History test save in progress'));
  assert.equal(store.getSnapshot().location.kind, 'settings');
  remove();
  await ui.act(async () => {
    window.dispatchEvent(new window.WheelEvent('wheel', { deltaX: -120 }));
    store.navigate({ kind: 'teams' });
    await new Promise((resolve) => setTimeout(resolve, 160));
  });
  assert.equal(store.getSnapshot().location.kind, 'teams');
});
