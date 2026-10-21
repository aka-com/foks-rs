/** First run as a setup assistant: Return continues, each step takes the focus. */

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
test.afterEach(() => {
  ui.cleanup();
  window.localStorage.clear();
});
test.after(async () => {
  await vite.close();
});

async function harness(overrides: Partial<Bridge>) {
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
  const {
    initialFirstRun,
    encodeFirstRunCheckpoint,
    FIRST_RUN_CHECKPOINT_KEY,
  } = (await vite.ssrLoadModule(
    '/src/first-run-state.ts',
  )) as typeof import('../src/first-run-state');
  const snapshot = {
    ...FIXTURE,
    servers: [],
    accounts: [],
    stores: [],
    storeInventory: [],
    profileInventory: [],
    catalogProfiles: [],
    items: [],
    parties: [],
    federation: [],
    groupDetailFailures: [],
    notifications: [],
    observedExpiredLeases: [],
    plaintext: {},
  };
  const store = new LocationStore({
    ...INITIAL_STATE,
    location: { kind: 'first-run', path: 'own', step: 'who' },
  });
  window.localStorage.setItem(
    FIRST_RUN_CHECKPOINT_KEY,
    encodeFirstRunCheckpoint(initialFirstRun('own', 'who')),
  );
  const bridge = {
    ...mockBridge(snapshot),
    native: true,
    firstRunFixture: undefined,
    discoverGoProfiles: async () => ({ installed: false, candidates: [] }),
    ...overrides,
  };
  return ui.render(createElement(App, { snapshot, store, bridge }));
}

const focused = (): Element | null => document.activeElement;

test('Return continues from the chosen radio and from the address field; each step takes the focus', async () => {
  const checked: string[] = [];
  const view = await harness({
    checkAndAddProfile: async (profile, probe) => {
      checked.push(probe);
      return {
        profile,
        hostId: `02${'7'.repeat(64)}`,
        lookupName: probe,
        canonicalName: probe,
        acceptance: 'inserted',
        chain: 1,
        epoch: 1,
      };
    },
  });
  await view.findByText('How are you joining?');
  // The first choice is made for the user and holds the focus.
  const own = view.getByRole('radio', { name: /Set up my own account/ });
  assert.equal(own.getAttribute('aria-checked'), 'true');
  await ui.waitFor(() => assert.equal(focused(), own));
  ui.fireEvent.keyDown(own, { key: 'Enter' });

  const address = (await view.findByLabelText(
    'Server address',
  )) as HTMLInputElement;
  await ui.waitFor(() => assert.equal(focused(), address));
  ui.fireEvent.change(address, { target: { value: 'foks.example.net' } });
  ui.fireEvent.keyDown(address, { key: 'Enter' });
  await ui.waitFor(() => assert.deepEqual(checked, ['foks.example.net']));
  await view.findByText(/foks\.example\.net verified/);
  // A modified Return is not the default button.
  ui.fireEvent.keyDown(focused() ?? document.body, {
    key: 'Enter',
    metaKey: true,
  });
  assert.equal(view.queryByText('Set up your account'), null);
  ui.fireEvent.keyDown(focused() ?? document.body, { key: 'Enter' });
  await view.findByText('Set up your account');
  await ui.waitFor(() =>
    assert.equal(focused()?.getAttribute('aria-label'), 'Your name'),
  );
});

test('a failed server check returns the focus to the address field, marked invalid', async () => {
  const view = await harness({
    checkAndAddProfile: async () => {
      throw new Error('connection refused');
    },
  });
  await view.findByText('How are you joining?');
  ui.fireEvent.keyDown(
    view.getByRole('radio', { name: /Set up my own account/ }),
    { key: 'Enter' },
  );
  const address = (await view.findByLabelText(
    'Server address',
  )) as HTMLInputElement;
  ui.fireEvent.change(address, { target: { value: 'foks.example.net' } });
  ui.fireEvent.keyDown(address, { key: 'Enter' });
  const alert = await view.findByRole('alert');
  await ui.waitFor(() => {
    const field = view.getByLabelText('Server address');
    assert.equal(focused(), field);
    assert.equal(field.getAttribute('aria-invalid'), 'true');
    assert.equal(field.getAttribute('aria-describedby'), alert.id);
  });
});
