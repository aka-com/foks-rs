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
  )) as typeof import('../kit/toasts');
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
    assert.ok(rendered.getByText('Select an FOKS account')),
  );
  assert.ok(
    scans >= 2,
    'Strict Mode replays discovery after cancelling its first effect',
  );
  ui.fireEvent.click(rendered.getByRole('radio', { name: /cli-owner/ }));
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Continue' }));
  ui.fireEvent.click(
    rendered.getByRole('button', { name: 'Use the official FOKS server' }),
  );
  assert.equal(
    (rendered.getByLabelText('Server address') as HTMLInputElement).value,
    'foks.app:4430',
  );
  ui.fireEvent.click(rendered.getByRole('button', { name: /Use this server/ }));
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
  )) as typeof import('../kit/overlay-primitives');
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
    assert.ok(rendered.getByRole('radio', { name: /cli-owner/ })),
  );
  ui.fireEvent.click(rendered.getByRole('radio', { name: /cli-owner/ }));
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
      'Server verification failed: the server does not match the selected CLI account.',
    ),
  );
  assert.equal(rendered.queryByText('Pair this Mac'), null);
});

for (const refreshFails of [false, true]) {
  test(`reconciles uncertain initialization before profile selection (refresh fails: ${refreshFails})`, async () => {
    const { FirstRunExperience } = (await vite.ssrLoadModule(
      '/src/screens/first-run-screen.tsx',
    )) as typeof import('../src/screens/first-run-screen.tsx');
    const { ToastProvider, ToastController } = (await vite.ssrLoadModule(
      '/kit/toasts.tsx',
    )) as typeof import('../kit/toasts.tsx');
    const { FIXTURE } = (await vite.ssrLoadModule(
      '/src/fixture.ts',
    )) as typeof import('../src/fixture.ts');
    const { mockBridge } = (await vite.ssrLoadModule(
      '/src/mock-bridge.ts',
    )) as typeof import('../src/mock-bridge.ts');
    let refreshes = 0;
    let pendingReads = 0;
    const bridge: Bridge = {
      ...mockBridge(),
      native: true,
      firstRunFixture: undefined,
      initializeClientState: async () => {
        throw {
          code: 'ambiguous',
          message: 'Initialization timed out',
          retryable: false,
          ambiguous: true,
          fatal: false,
        };
      },
      discoverGoProfiles: async () => ({ installed: false, candidates: [] }),
      listPendingOperations: async () => {
        pendingReads++;
        return [];
      },
    };
    const rendered = ui.render(
      createElement(ToastProvider, {
        controller: new ToastController(),
        children: createElement(FirstRunExperience, {
          bridge,
          world: FIXTURE,
          location: { kind: 'first-run', path: 'own', step: 'who' },
          onNavigate: () => {},
          automaticEntry: true,
          onRefreshWorld: async () => {
            refreshes++;
            if (refreshFails) throw new Error('Catalog unavailable');
            return FIXTURE;
          },
          concealSignal: 0,
        }),
      }),
    );
    await ui.waitFor(() => assert.equal(refreshes, 1));
    await ui.waitFor(() =>
      assert.ok(
        rendered.getByText(
          refreshFails
            ? /FOKS could not finish checking the vault/
            : /FOKS refreshed the vault after an uncertain result/,
        ),
      ),
    );
    assert.equal(
      pendingReads,
      0,
      'pending operations require a selected profile',
    );
    if (refreshFails)
      assert.equal(rendered.queryByText(/FOKS refreshed the vault/), null);
  });
}

test('first-run account navigation, server edits, and connection errors stay scoped', async () => {
  const { FirstRunExperience } = (await vite.ssrLoadModule(
    '/src/screens/first-run-screen.tsx',
  )) as typeof import('../src/screens/first-run-screen.tsx');
  const { ToastProvider, ToastController } = (await vite.ssrLoadModule(
    '/kit/toasts.tsx',
  )) as typeof import('../kit/toasts.tsx');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture.ts');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge.ts');
  const failure = (message: string) => ({
    code: 'invalid-request',
    message,
    fatal: false,
    ambiguous: false,
    retryable: false,
  });
  const navigations: unknown[] = [];
  const bridge: Bridge = {
    ...mockBridge(),
    native: true,
    discoverGoProfiles: async () => ({
      installed: true,
      candidates: [candidate],
    }),
    checkAndAddGoProfile: async (_candidate, _host, profile, address) => ({
      profile,
      hostId: candidate.hostId,
      lookupName: address,
      canonicalName: 'foks.app',
      acceptance: 'unchanged',
      chain: 1,
      epoch: 1,
    }),
    copyGoProfileDevice: async () => {
      throw failure('Copy failed');
    },
    resumeGoProfilePairing: async () => {
      throw failure(
        'Refresh the setup status before resuming this operation.',
      );
    },
    recoverOwnerAccount: async () => {
      throw failure('Recovery failed');
    },
  };
  const view = ui.render(
    createElement(ToastProvider, {
      controller: new ToastController(),
      children: createElement(FirstRunExperience, {
        bridge,
        world: FIXTURE,
        location: { kind: 'first-run', path: 'own', step: 'who' },
        concealSignal: 0,
        onNavigate: (location: unknown) => {
          navigations.push(location);
        },
        onRefreshWorld: async () => FIXTURE,
      }),
    }),
  );
  await view.findByRole('radio', { name: /cli-owner/ });
  ui.fireEvent.click(
    view.getByRole('button', { name: 'Create a new account' }),
  );
  assert.ok(view.queryByText(/Already using FOKS/) === null);
  assert.ok(view.queryByText(/You’ll need/) === null);
  ui.fireEvent.click(view.getByRole('button', { name: 'Back' }));
  ui.fireEvent.click(view.getByRole('radio', { name: /cli-owner/ }));
  ui.fireEvent.click(view.getByRole('button', { name: 'Continue' }));
  ui.fireEvent.click(
    view.getByRole('button', { name: 'Use the official FOKS server' }),
  );
  ui.fireEvent.click(view.getByRole('button', { name: 'Use this server' }));
  await view.findByRole('button', { name: 'Details' });
  assert.ok(view.queryByText('Pinned on this Mac') === null);
  const addressField = view.getByLabelText('Server address');
  addressField.focus();
  ui.fireEvent.change(addressField, { target: { value: 'changed.example' } });
  assert.equal(document.activeElement, view.getByLabelText('Server address'));
  assert.ok(view.queryByRole('button', { name: 'Details' }) === null);
  assert.ok(view.queryByRole('button', { name: 'Continue' }) === null);
  ui.fireEvent.click(view.getByRole('button', { name: 'Use this server' }));
  await view.findByRole('button', { name: 'Continue' });
  ui.fireEvent.click(view.getByRole('button', { name: 'Continue' }));
  const titles = [...view.container.querySelectorAll('.pcard h3')].map(
    (el) => el.textContent,
  );
  assert.deepEqual(titles, [
    'Recover with your backup phrase',
    'Copy this Mac’s CLI device',
    'Use the CLI to approve this as a new device',
  ]);
  ui.fireEvent.click(
    view.getByRole('button', { name: 'Copy existing device' }),
  );
  const copyError = await view.findByText('Copy failed');
  assert.equal(
    copyError.closest('.pcard')?.querySelector('h3')?.textContent,
    'Copy this Mac’s CLI device',
  );
  ui.fireEvent.click(view.getByRole('button', { name: 'Resume pairing' }));
  const pairError = await view.findByText(
    'Refresh the setup status before resuming this operation.',
  );
  assert.equal(
    pairError.closest('.pcard')?.querySelector('h3')?.textContent,
    'Use the CLI to approve this as a new device',
  );
  ui.fireEvent.change(view.getByLabelText('Backup phrase'), {
    target: { value: 'one two three' },
  });
  ui.fireEvent.click(view.getByRole('button', { name: 'Recover' }));
  const recoverError = await view.findByText('Recovery failed');
  assert.equal(
    recoverError.closest('.pcard')?.querySelector('h3')?.textContent,
    'Recover with your backup phrase',
  );
  ui.fireEvent.click(
    view.getByRole('button', { name: 'Create a new account' }),
  );
  assert.ok(view.getByPlaceholderText('yourname'));
  assert.ok(view.getByPlaceholderText('Your Mac'));
  ui.fireEvent.click(view.getByRole('button', { name: 'Back' }));
  assert.ok(view.getByText('Add this Mac to your account'));
  ui.fireEvent.click(view.getByRole('button', { name: 'Leave setup' }));
  assert.deepEqual(navigations.at(-1), { kind: 'all' });
});

test('personal recovery puts backup first and completes without creating a group', async () => {
  const { FirstRunExperience } = (await vite.ssrLoadModule(
    '/src/screens/first-run-screen.tsx',
  )) as typeof import('../src/screens/first-run-screen');
  const { ToastProvider, ToastController } = (await vite.ssrLoadModule(
    '/kit/toasts.tsx',
  )) as typeof import('../kit/toasts');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  window.history.replaceState(null, '', '/?state=protect&path=own');
  let creations = 0;
  const bridge: Bridge = {
    ...mockBridge(),
    createGroup: async () => {
      creations++;
      throw new Error('Onboarding must not create groups');
    },
  };
  const view = ui.render(
    createElement(ToastProvider, {
      controller: new ToastController(),
      children: createElement(FirstRunExperience, {
        bridge,
        world: FIXTURE,
        location: { kind: 'first-run', path: 'own', step: 'protect' },
        onNavigate: () => {},
        onRefreshWorld: async () => FIXTURE,
        concealSignal: 0,
      }),
    }),
  );
  assert.deepEqual(
    [...view.container.querySelectorAll('.pcard h3')].map(
      (el) => el.textContent,
    ),
    ['Backup phrase', 'Passphrase'],
  );
  assert.ok(view.queryByText(/YubiKey/) === null);
  assert.ok(view.queryByText('Create a group') === null);
  ui.fireEvent.click(view.getByRole('button', { name: 'Continue' }));
  await view.findByRole('button', { name: 'Open Personal' });
  assert.ok(view.queryByText('Create a group') === null);
  assert.equal(creations, 0);
  const checkpoint = JSON.parse(
    window.localStorage.getItem('foks.first-run.v1') ?? '{}',
  ) as { state?: string; group?: unknown };
  assert.equal(checkpoint.state, 'checklist-own');
  assert.equal(checkpoint.group, undefined);
  window.history.replaceState(null, '', '/');
});
