/**
 * Navigation behavior when an external action attempts to leave setup. Steps
 * containing secrets block external navigation; setup's own navigation remains
 * allowed; Leave setup clears entered values.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import type { Bridge } from '../src/bridge';
import type { FirstRunCheckpoint } from '../src/first-run-state';
import type { FirstRunExperienceProps } from '../src/screens/first-run-screen';
import type { GuardVerdict, Location } from '../src/location';
import { installDom } from './lib/dom-harness';

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
test.afterEach(() => {
  ui.cleanup();
  window.localStorage.clear();
});
test.after(async () => vite.close());

const AWAY: Location = { kind: 'all' };

async function harness() {
  const screen = (await vite.ssrLoadModule(
    '/src/screens/first-run-screen.tsx',
  )) as typeof import('../src/screens/first-run-screen');
  const locations = (await vite.ssrLoadModule(
    '/src/location.ts',
  )) as typeof import('../src/location');
  const guards = (await vite.ssrLoadModule(
    '/src/navigation-guard.tsx',
  )) as typeof import('../src/navigation-guard');
  const state = (await vite.ssrLoadModule(
    '/src/first-run-state.ts',
  )) as typeof import('../src/first-run-state');
  const { ToastController, ToastProvider } = (await vite.ssrLoadModule(
    '/kit/toasts.tsx',
  )) as typeof import('../kit/toasts');
  const { OverlayProvider } = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);

  const profile = {
    profile: 'personal',
    hostId: `02${'2'.repeat(64)}`,
    lookupName: 'localhost',
    canonicalName: 'localhost',
    acceptance: 'inserted' as const,
    chain: 1,
    epoch: 1,
  };
  const snapshot = {
    ...FIXTURE,
    accounts: [],
    stores: [],
    profileInventoryStatus: 'unavailable' as const,
    profileInventory: [
      {
        profile: 'personal',
        accounts: 'unavailable' as const,
        teams: 'unavailable' as const,
      },
    ],
  };
  const bridge: Bridge = {
    ...mockBridge(snapshot),
    native: true,
    firstRunFixture: undefined,
  };
  const checkpointAt = (
    step: FirstRunCheckpoint['state'],
  ): FirstRunCheckpoint => ({
    ...state.initialFirstRun('own', step),
    // Not the managed local server: that path's recovery step offers the
    // backup phrase alone, and the passphrase this test types is on the step
    // an account on a configured server reaches.
    managedLocal: false,
    serverAddress: 'localhost:4430',
    profile,
    account: { alias: 'personal', username: 'satoshi', deviceName: 'Mac' },
  });

  /** Setup under a location store of its own, wired as the shell wires it. */
  const render = (step: FirstRunCheckpoint['state']) => {
    const here: Location = { kind: 'first-run', path: 'own', step };
    const store = new locations.LocationStore({
      ...locations.INITIAL_STATE,
      location: here,
    });
    const refusals: string[] = [];
    store.setRefusalHandler((reason) => refusals.push(reason));
    window.localStorage.setItem(
      state.FIRST_RUN_CHECKPOINT_KEY,
      state.encodeFirstRunCheckpoint(checkpointAt(step)),
    );
    const props: FirstRunExperienceProps = {
      bridge,
      snapshot,
      location: here,
      onNavigate: (location) => store.navigate(location),
      onRefreshSnapshot: async () => snapshot,
      concealSignal: 0,
      agentReady: true,
    };
    const view = ui.render(
      createElement(OverlayProvider, {
        backgroundRef: { current: null },
        portalRoot,
        children: createElement(ToastProvider, {
          controller: new ToastController(),
          children: createElement(guards.NavigationGuardProvider, {
            store,
            children: createElement(screen.FirstRunExperience, props),
          }),
        }),
      }),
    );
    return { store, refusals, view };
  };

  return {
    render,
    verdict: (
      store: InstanceType<typeof locations.LocationStore>,
    ): GuardVerdict =>
      store.navigationVerdict({ kind: 'navigate', location: AWAY }),
    within: (
      store: InstanceType<typeof locations.LocationStore>,
    ): GuardVerdict =>
      store.navigationVerdict({
        kind: 'navigate',
        location: { kind: 'first-run', step: 'phrase', path: 'own' },
      }),
  };
}

test('a setup step with nothing typed is not in the way', async () => {
  const h = await harness();
  const { store } = h.render('protect');
  await ui.act(async () => {
    await Promise.resolve();
  });
  assert.equal(h.verdict(store), null);
});

test('a typed passphrase refuses a move out of setup, but not within it', async () => {
  const h = await harness();
  const { store, refusals, view } = h.render('protect');
  const field = await view.findByLabelText('Passphrase');
  await ui.act(async () => {
    ui.fireEvent.change(field, { target: { value: 'correct horse' } });
  });

  assert.deepEqual(h.verdict(store), {
    verdict: 'refuse',
    reason: 'Finish or leave setup first.',
  });
  // Setup steps share one guarded workflow, so navigation between those steps
  // remains allowed.
  assert.equal(h.within(store), null);

  await ui.act(async () => {
    store.navigate(AWAY);
    await Promise.resolve();
  });
  assert.deepEqual(store.getSnapshot().location, {
    kind: 'first-run',
    path: 'own',
    step: 'protect',
  });
  assert.deepEqual(refusals, ['Finish or leave setup first.']);
});

test('Leave setup clears what was typed and goes', async () => {
  const h = await harness();
  const { store, view } = h.render('protect');
  const field = await view.findByLabelText('Passphrase');
  await ui.act(async () => {
    ui.fireEvent.change(field, { target: { value: 'correct horse' } });
  });
  assert.equal(h.verdict(store)?.verdict, 'refuse');

  await ui.act(async () => {
    ui.fireEvent.click(view.getByRole('button', { name: 'Finish later' }));
    await Promise.resolve();
  });
  assert.deepEqual(store.getSnapshot().location, AWAY);
  assert.equal(h.verdict(store), null);
});

test('completed setup steps do not register navigation guards', async () => {
  const h = await harness();
  const { store } = h.render('checklist-own');
  await ui.act(async () => {
    await Promise.resolve();
  });
  assert.equal(h.verdict(store), null);
});
