import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, StrictMode } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import type { Bridge, GoProfileCandidate } from '../src/bridge';
import { installDom } from './lib/dom-harness';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
});
let ui: typeof import('@testing-library/react');
let vite: ViteDevServer;
const candidate = {
  candidateId: 'candidate',
  username: 'cli-owner',
  hostId: '02' + 'ab'.repeat(32),
  userId: '01' + 'cd'.repeat(32),
  deviceId: '03' + 'ef'.repeat(32),
  role: 'owner',
  storageKind: 'macos-keychain',
  hidden: false,
  provisional: false,
  pairable: true,
  copyable: true,
} satisfies GoProfileCandidate;
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

test('discovers Go CLI profile in StrictMode and passes profile credentials to server check', async () => {
  const { FirstRunExperience } = (await vite.ssrLoadModule(
    '/src/screens/first-run-screen.tsx',
  )) as typeof import('../src/screens/first-run-screen');
  const { ToastProvider, ToastController } = (await vite.ssrLoadModule(
    '/kit/toasts.tsx',
  )) as typeof import('../../ui/kit/toasts');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  let scans = 0;
  const checks: unknown[][] = [];
  const bridge: Bridge = {
    ...mockBridge(),
    native: true,
    discoverGoProfiles: async () => {
      scans++;
      return { installed: true, candidates: [candidate] };
    },
    checkAndAddGoProfile: async (...args: unknown[]) => {
      checks.push(args);
      throw new Error('deliberate host mismatch');
    },
  };
  const rendered = ui.render(
    createElement(
      StrictMode,
      null,
      createElement(ToastProvider, {
        controller: new ToastController(),
        children: createElement(FirstRunExperience, {
          bridge,
          world: FIXTURE,
          location: { kind: 'first-run', path: 'own', step: 'who' },
          onNavigate: () => {},
          onRefreshWorld: async () => FIXTURE,
          concealSignal: 0,
        }),
      }),
    ),
  );
  await ui.waitFor(() =>
    assert.ok(rendered.getByText('FOKS is already set up on this Mac')),
  );
  assert.ok(
    scans >= 2,
    'Strict Mode replays discovery after cancelling its first effect',
  );
  ui.fireEvent.click(rendered.getByRole('button', { name: /cli-owner/ }));
  ui.fireEvent.click(
    rendered.getByRole('button', { name: 'Connect selected account' }),
  );
  ui.fireEvent.click(
    rendered.getByRole('button', { name: 'Use official FOKS server' }),
  );
  assert.equal(
    (rendered.getByLabelText('Server address') as HTMLInputElement).value,
    'foks.app:4430',
  );
  ui.fireEvent.click(
    rendered.getByRole('button', { name: /Check the server/ }),
  );
  await ui.waitFor(() => assert.equal(checks.length, 1));
  assert.equal(checks[0][0], candidate.candidateId);
  assert.equal(checks[0][1], candidate.hostId);
  assert.equal(checks[0][3], 'foks.app:4430');
});

test('disables account selection and dialog dismissal while server verification is pending', async () => {
  const { GoProfileConnectSheet } = (await vite.ssrLoadModule(
    '/src/screens/go-profile-connect.tsx',
  )) as typeof import('../src/screens/go-profile-connect');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  let resolve!: (
    value: Awaited<ReturnType<Bridge['checkAndAddGoProfile']>>,
  ) => void;
  let closed = false;
  const bridge: Bridge = {
    ...mockBridge(),
    discoverGoProfiles: async () => ({
      installed: true,
      candidates: [candidate],
    }),
    checkAndAddGoProfile: () =>
      new Promise((done) => {
        resolve = done;
      }),
  };
  const { OverlayProvider } = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../../ui/kit/overlay-primitives');
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  const rendered = ui.render(
    createElement(OverlayProvider, {
      backgroundRef: { current: null },
      portalRoot,
      children: createElement(GoProfileConnectSheet, {
        bridge,
        onClose: () => {
          closed = true;
        },
        onConnected: async () => {},
        onError: () => {},
      }),
    }),
  );
  await ui.waitFor(() =>
    assert.ok(rendered.getByRole('button', { name: /cli-owner/ })),
  );
  ui.fireEvent.click(rendered.getByRole('button', { name: /cli-owner/ }));
  ui.fireEvent.click(
    rendered.getByRole('button', { name: 'Use official FOKS server' }),
  );
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Check server' }));
  assert.equal(
    (rendered.getByLabelText('Server address') as HTMLInputElement).disabled,
    true,
  );
  assert.equal(
    (
      rendered.getByRole('button', {
        name: 'Choose another account',
      }) as HTMLButtonElement
    ).disabled,
    true,
  );
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Cancel' }));
  ui.fireEvent.keyDown(document, { key: 'Escape' });
  assert.equal(closed, false);
  await ui.act(async () => {
    resolve({
      profile: 'cli-owner',
      acceptance: 'unchanged',
      lookupName: 'foks.app',
      canonicalName: 'foks.app',
      hostId: 'wrong-host',
      chain: 1,
      epoch: 1,
    });
  });
  assert.ok(
    rendered.getByText(
      'The checked server does not match the selected CLI profile.',
    ),
  );
  assert.equal(rendered.queryByText('Pair this Mac'), null);
});
