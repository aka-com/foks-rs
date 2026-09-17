import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import type { StoreRef, AgentSnapshot } from '../src/model';
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

test.afterEach(() => ui.cleanup());
test.after(async () => vite.close());

async function renderKeys(
  snapshot: AgentSnapshot,
  store?: StoreRef,
  scene = 'settings',
  section: 'keys' | 'macs' = 'keys',
) {
  const { SettingsScreen } = (await vite.ssrLoadModule(
    '/src/screens/settings-screen.tsx',
  )) as typeof import('../src/screens/settings-screen');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const { ToastController, ToastProvider } = (await vite.ssrLoadModule(
    '/kit/toasts.tsx',
  )) as typeof import('../kit/toasts');
  const { OverlayProvider } = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives');
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  const rendered = ui.render(
    createElement(OverlayProvider, {
      backgroundRef: { current: null },
      portalRoot,
      children: createElement(ToastProvider, {
        controller: new ToastController(),
        children: createElement(SettingsScreen, {
          snapshot,
          bridge: mockBridge(snapshot),
          variant: 'devices',
          location: {
            kind: 'devices',
            section,
            ...(store ? { store } : {}),
          },
          scene,
          onNavigate: () => {},
          onRefresh: async () => {},
          onRefreshSnapshot: async () => snapshot,
          onError: (error: unknown) => {
            throw error;
          },
          onMutationError: async (error: unknown) => {
            throw error;
          },
          onLock: async () => true,
          agentLifecycle: { state: 'ready' },
          onRetryAgent: async () => {},
        }),
      }),
    }),
  );
  await ui.act(async () => {
    await Promise.resolve();
  });
  return rendered;
}

test('security-key settings explain that a fresh installation needs an account', async () => {
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const empty: AgentSnapshot = {
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
  const rendered = await renderKeys(empty);

  assert.ok(rendered.getByText('No account configured'));
  assert.ok(
    rendered.getByText(
      'Add or recover an account before configuring security keys.',
    ),
  );
  assert.equal(rendered.queryByText('Security-key access is stopped'), null);
});

test('security-key settings retain recovery guidance for a stopped account', async () => {
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const account = FIXTURE.stores.find(
    (candidate) => candidate.kind === 'account',
  );
  assert.ok(account?.kind === 'account');
  const blocked: AgentSnapshot = {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server) =>
      server.id === account.server
        ? {
            ...server,
            trust: {
              status: 'blocked' as const,
              error: {
                code: 'verification-failed',
                message: 'Pinned server history changed.',
                retryable: false,
                ambiguous: false,
                fatal: true,
              },
            },
          }
        : server,
    ),
  };
  const rendered = await renderKeys(blocked, account.id);

  assert.ok(rendered.getByText('Security-key access is stopped'));
  assert.ok(
    rendered.getByText(
      'Restore account access in Server settings before changing these settings.',
    ),
  );
  assert.equal(rendered.queryByText('No account configured'), null);
});

test('the sheet scenes still open on Devices, where those panes live now', async () => {
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const rendered = await renderKeys(FIXTURE, 'acct:personal', 'settings-enrol');
  const dialog = await ui.waitFor(() => rendered.getByRole('dialog'));
  assert.equal(
    ui.within(dialog).getByRole('heading', { level: 2 }).textContent,
    'Create a YubiKey account',
  );
});

test('the backup-phrase scene opens its sheet on the recovery pane', async () => {
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  // `?state=settings-phrase` and `?state=settings&section=phrase` both land on
  // Devices' recovery pane, and the scene still opens the one-time sheet.
  const rendered = await renderKeys(
    FIXTURE,
    'acct:personal',
    'settings-phrase',
    'macs',
  );
  const dialog = await ui.waitFor(() => rendered.getByRole('dialog'));
  assert.equal(
    ui.within(dialog).getByRole('heading', { level: 2 }).textContent,
    'Save backup phrase',
  );
});
