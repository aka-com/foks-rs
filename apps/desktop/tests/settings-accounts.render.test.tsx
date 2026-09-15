/**
 * Verifies the Settings → Accounts inset opens each account workflow as a
 * sheet, and that Recovery devices switches accounts through its picker.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import type { Location, SettingsSection } from '../src/location';
import type { AgentSnapshot } from '../src/model';
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

async function renderSettings(
  section: SettingsSection,
  onNavigate: (location: Location) => void = () => {},
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
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const snapshot: AgentSnapshot = FIXTURE;
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
          location: { kind: 'settings', section, store: 'acct:personal' },
          scene: 'settings',
          onNavigate,
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
  return { rendered, snapshot };
}

test('every account row offers its workflow and the passphrase row drops Verify', async () => {
  const { rendered } = await renderSettings('account');

  // Two fixture accounts; "Change…" is on both the username and passphrase
  // rows, so it appears twice per account.
  for (const [name, count] of [
    ['Change…', 4],
    ['Set…', 2],
    ['Sign in…', 2],
    ['Manage…', 2],
    ['Open…', 2],
    ['Join a group…', 2],
  ] as const)
    assert.equal(
      rendered.getAllByRole('button', { name }).length,
      count,
      `unexpected number of "${name}" buttons`,
    );
  assert.equal(rendered.queryByRole('button', { name: 'Verify' }), null);
  assert.ok(rendered.getAllByText('Local alias').length);
  assert.equal(rendered.queryByText('Organization'), null);
  assert.ok(rendered.getAllByText('SSO').length);
});

test('the username row opens a sheet titled for the workflow, not the account', async () => {
  const { rendered } = await renderSettings('account');

  await ui.act(async () => {
    ui.fireEvent.click(rendered.getAllByRole('button', { name: 'Change…' })[0]);
  });
  const dialog = await ui.waitFor(() => rendered.getByRole('dialog'));
  const heading = ui.within(dialog).getByRole('heading', { level: 2 });
  assert.equal(heading.textContent, 'Change username');
  assert.ok(ui.within(dialog).getByText('satoshi on foks.example.net'));
  assert.ok(ui.within(dialog).getByLabelText('Username'));
});

test('the recovery-devices picker lists every account and switching navigates', async () => {
  const chosen: Location[] = [];
  const { rendered } = await renderSettings('macs', (location) =>
    chosen.push(location),
  );

  const trigger = rendered.getByRole('button', { name: 'This account' });
  assert.ok(trigger.textContent?.includes('satoshi on foks.example.net'));
  await ui.act(async () => {
    ui.fireEvent.click(trigger);
  });
  const listbox = await ui.waitFor(() =>
    rendered.getByRole('listbox', { name: 'This account' }),
  );
  const items = ui.within(listbox).getAllByRole('option');
  assert.equal(items.length, 2);
  assert.ok(items[0].textContent?.includes('satoshi on foks.example.net'));
  assert.ok(items[1].textContent?.includes('vitalik on foks.acme-corp.com'));

  await ui.act(async () => {
    ui.fireEvent.click(items[1]);
  });
  assert.deepEqual(chosen.at(-1), {
    kind: 'settings',
    section: 'macs',
    store: 'acct:work',
  });
});

test('organization sign-in survives browser focus and can finish the same flow', async () => {
  const { rendered } = await renderSettings('account');
  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getAllByRole('button', { name: 'Sign in…' })[0],
    );
  });
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Sign in' }));
  });
  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'Open sign-in browser' }),
    );
    window.dispatchEvent(new window.Event('blur'));
  });
  assert.ok(rendered.getByRole('dialog'));
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Check sign-in' }));
  });
  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'Finish sign-in' }),
    );
  });
  assert.ok(
    rendered.getByText('Account authentication and service access verified.'),
  );
});
