/**
 * The Teams tab under a real `LocationStore`.
 *
 * The address can arrive carrying a sheet intent (`{kind:'teams',
 * open:'create'|'join'}`). The page opens that sheet once and replaces the
 * address with the plain list. The sheets themselves are tab-persisted state,
 * so the interesting part is what the store holds afterwards: closing the
 * sheet must clear the saved record, or the Teams tab would restore it the
 * next time the reader comes back to it.
 *
 * `navigation-store.test.ts` covers the store's own half of this — the
 * resumed address never carries `open`. Here the screen is mounted on top of
 * it, which is where the saved sheet record is written.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactNode } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import { installDom } from './lib/dom-harness';
import type { Location, LocationStore, NavigateOptions } from '../src/location';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
  act: true,
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
test.after(async () => {
  await vite.close();
});

/** The Teams tab mounted over a store, with every other tab a placeholder. */
async function setup(
  start: Location,
  /** The fixture scene, which `?state=create` sets alongside the address. */
  scene = 'groups',
): Promise<{ store: LocationStore }> {
  const locations = (await vite.ssrLoadModule(
    '/src/location.ts',
  )) as typeof import('../src/location');
  const { TeamsScreen } = (await vite.ssrLoadModule(
    '/src/screens/teams-screen.tsx',
  )) as typeof import('../src/screens/teams-screen');
  const { NavigationGuardProvider } = (await vite.ssrLoadModule(
    '/src/navigation-guard.tsx',
  )) as typeof import('../src/navigation-guard');
  const { OverlayProvider } = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives');
  const { ToastController, ToastProvider } = (await vite.ssrLoadModule(
    '/kit/toasts.tsx',
  )) as typeof import('../kit/toasts');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');

  const snapshot = FIXTURE;
  const bridge = mockBridge(snapshot);
  const store = new locations.LocationStore();
  store.setAccountStores(snapshot.stores);
  store.navigate(start, { force: true });
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  const controller = new ToastController();

  function Host(): ReactNode {
    const state = locations.useLocationState(store);
    return createElement(OverlayProvider, {
      backgroundRef: { current: null },
      portalRoot: portalRoot as HTMLElement,
      children: createElement(ToastProvider, {
        controller,
        children: createElement(NavigationGuardProvider, {
          store,
          children:
            state.location.kind === 'teams'
              ? createElement(TeamsScreen, {
                  snapshot,
                  bridge,
                  location: state.location,
                  scene,
                  onNavigate: (next: Location, options?: NavigateOptions) =>
                    store.navigate(next, options),
                  onRefresh: async () => {},
                  onRefreshSnapshot: async () => snapshot,
                  onError: (error: unknown) => {
                    throw error;
                  },
                  onMutationError: async (error: unknown) => {
                    throw error;
                  },
                })
              : createElement('p', null, 'Elsewhere'),
        }),
      }),
    });
  }

  ui.render(createElement(Host));
  await ui.act(async () => {
    await Promise.resolve();
  });
  return { store };
}

/** Leaves the Teams tab and comes back the way the rail does. */
async function roundTrip(store: LocationStore): Promise<void> {
  await ui.act(async () => {
    store.navigateTab('files');
  });
  await ui.act(async () => {
    store.navigateTab('teams');
  });
}

for (const { intent, heading } of [
  { intent: 'create' as const, heading: 'Create a team' },
  { intent: 'join' as const, heading: 'Join a team' },
]) {
  test(`a closed ${intent} sheet opened by the address does not come back with the tab`, async () => {
    const { store } = await setup({ kind: 'teams', open: intent });
    assert.ok(ui.screen.getByRole('heading', { name: heading }));
    // The intent is spent: the address is the plain list again.
    assert.equal(store.getSnapshot().location.kind, 'teams');
    assert.equal(
      (store.getSnapshot().location as { open?: string }).open,
      undefined,
    );

    // Escape reaches the sheet through the dialog it is drawn in.
    const dialog = document.querySelector('[role="dialog"]');
    assert.ok(dialog);
    await ui.act(async () => {
      ui.fireEvent.keyDown(dialog, { key: 'Escape' });
    });
    assert.equal(ui.screen.queryByRole('heading', { name: heading }), null);
    // The sheet was closed, so nothing about it is left to restore.
    assert.equal(store.getSnapshot().sheet, undefined);

    await roundTrip(store);
    assert.equal(ui.screen.queryByRole('heading', { name: heading }), null);
    assert.equal(store.getSnapshot().sheet, undefined);
  });
}

test('a link that names the scene and the sheet still spends the intent once', async () => {
  // `?state=create` sets the fixture scene and the address together. The
  // sheet must still be opened against the canonical address, so that closing
  // it leaves no saved record behind.
  const { store } = await setup({ kind: 'teams', open: 'create' }, 'create');
  assert.ok(ui.screen.getByRole('heading', { name: 'Create a team' }));
  assert.equal(
    (store.getSnapshot().location as { open?: string }).open,
    undefined,
  );
  const dialog = document.querySelector('[role="dialog"]');
  assert.ok(dialog);
  await ui.act(async () => {
    ui.fireEvent.keyDown(dialog, { key: 'Escape' });
  });
  assert.equal(
    ui.screen.queryByRole('heading', { name: 'Create a team' }),
    null,
  );
  assert.equal(store.getSnapshot().sheet, undefined);
});

test('a sheet the reader left open is still restored with the tab', async () => {
  const { store } = await setup({ kind: 'teams', open: 'create' });
  assert.ok(ui.screen.getByRole('heading', { name: 'Create a team' }));
  await roundTrip(store);
  assert.ok(ui.screen.getByRole('heading', { name: 'Create a team' }));
});
