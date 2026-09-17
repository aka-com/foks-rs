import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, StrictMode } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import type { Bridge, CommandError, GoProfileCandidate } from '../src/bridge';
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

async function discoveryRecoveryHarness() {
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
  const world = {
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
  const checkpoint = encodeFirstRunCheckpoint(initialFirstRun('own', 'who'));
  window.localStorage.setItem(FIRST_RUN_CHECKPOINT_KEY, checkpoint);
  const bridge = {
    ...mockBridge(world),
    native: true,
    firstRunFixture: undefined,
  };
  return {
    bridge,
    store,
    checkpoint: () => window.localStorage.getItem(FIRST_RUN_CHECKPOINT_KEY),
    render: (overrides: Partial<Bridge>) =>
      ui.render(
        createElement(App, {
          world,
          store,
          bridge: { ...bridge, ...overrides },
        }),
      ),
  };
}

function discoveryFailure(code: string): CommandError {
  return {
    code,
    message: `Discovery failed: ${code}`,
    retryable: false,
    ambiguous: false,
    fatal: false,
    details: { reason: 'initialize-state' },
  };
}

for (const notifyGlobally of [false, true]) {
  test(`bootstrap discovery failure recovers through shared setup (${notifyGlobally ? 'bridge and callback' : 'callback only'})`, async () => {
    const harness = await discoveryRecoveryHarness();
    const { tauriBridge, onAgentReadinessRequired } = (await vite.ssrLoadModule(
      '/src/bridge.ts',
    )) as typeof import('../src/bridge');
    let discoveries = 0;
    let initializations = 0;
    let notifications = 0;
    let ready = false;
    let releaseCatalog: (() => void) | undefined;
    const catalogGate = new Promise<void>((resolve) => {
      releaseCatalog = resolve;
    });
    const failure = discoveryFailure('bootstrap-required');
    const nativeWindow = window as unknown as { __TAURI_INTERNALS__?: unknown };
    const previous = nativeWindow.__TAURI_INTERNALS__;
    nativeWindow.__TAURI_INTERNALS__ = {
      invoke: async () => {
        throw failure;
      },
    };
    const stop = onAgentReadinessRequired(() => {
      notifications++;
    });
    try {
      const rendered = harness.render({
        discoverGoProfiles: async () => {
          discoveries++;
          if (!ready) {
            if (notifyGlobally) return tauriBridge.discoverGoProfiles();
            throw failure;
          }
          return { installed: true, candidates: [candidate] };
        },
        agentStatus: async () =>
          ready
            ? { state: 'ready' }
            : { state: 'bootstrap', step: 'initialize-state' },
        initializeClientState: async () => {
          initializations++;
          ready = true;
          return { state: 'ready' };
        },
        listCatalog: async () => {
          await catalogGate;
          return harness.bridge.listCatalog();
        },
      });
      await ui.waitFor(() =>
        assert.ok(rendered.getByText('Finish preparing this Mac')),
      );
      const saved = harness.checkpoint();
      assert.ok(saved);
      assert.equal(initializations, 0, 'recovery waits for Retry setup');
      assert.equal(
        rendered.queryByText(/Could not check existing CLI profiles/),
        null,
      );
      ui.fireEvent.click(rendered.getByRole('button', { name: 'Retry setup' }));
      await ui.waitFor(() => assert.equal(initializations, 1));
      assert.equal(
        discoveries,
        1,
        'discovery waits for the refreshed catalog too',
      );
      assert.ok(rendered.getByText('Finish preparing this Mac'));
      assert.equal(harness.checkpoint(), saved);
      await ui.act(async () => {
        releaseCatalog?.();
      });
      await ui.waitFor(() =>
        assert.ok(rendered.getByText('Select an FOKS account')),
      );
      assert.equal(discoveries, 2);
      assert.equal(initializations, 1);
      assert.equal(notifications, notifyGlobally ? 1 : 0);
      assert.equal(harness.checkpoint(), saved);
      assert.equal(harness.store.getSnapshot().location.kind, 'first-run');
      assert.equal(
        rendered.queryByText(/Could not check existing CLI profiles/),
        null,
      );
    } finally {
      releaseCatalog?.();
      stop();
      if (previous === undefined) delete nativeWindow.__TAURI_INTERNALS__;
      else nativeWindow.__TAURI_INTERNALS__ = previous;
    }
  });
}

for (const code of ['agent-lost', 'version-mismatch']) {
  test(`${code} discovery failure reaches the lifecycle blocker`, async () => {
    const harness = await discoveryRecoveryHarness();
    const rendered = harness.render({
      discoverGoProfiles: async () => {
        throw discoveryFailure(code);
      },
    });
    await ui.waitFor(() =>
      assert.ok(rendered.getByText('Finish preparing this Mac')),
    );
    assert.equal(
      rendered.queryByText(/Could not check existing CLI profiles/),
      null,
    );
  });
}

for (const failure of [
  discoveryFailure('scan-failed'),
  new Error('Malformed discovery response'),
]) {
  test(`ordinary discovery error stays inline: ${failure.message}`, async () => {
    const harness = await discoveryRecoveryHarness();
    let initializations = 0;
    const rendered = harness.render({
      discoverGoProfiles: async () => {
        throw failure;
      },
      initializeClientState: async () => {
        initializations++;
        return { state: 'ready' };
      },
    });
    await ui.waitFor(() =>
      assert.ok(rendered.getByText(/Could not check existing CLI profiles/)),
    );
    assert.equal(rendered.queryByRole('button', { name: 'Retry setup' }), null);
    assert.equal(initializations, 0);
  });
}

test('joining choices continue without a next-steps module', async () => {
  const harness = await discoveryRecoveryHarness();
  const view = harness.render({
    discoverGoProfiles: async () => ({ installed: false, candidates: [] }),
  });
  await view.findByText('How are you joining?');
  for (const name of [/Set up my own account/, /Join an existing group/]) {
    ui.fireEvent.click(view.getByRole('radio', { name }));
    assert.equal(view.queryByText(/What happens next/i), null);
    assert.equal(
      (view.getByRole('button', { name: 'Continue' }) as HTMLButtonElement).disabled,
      false,
    );
  }
  ui.fireEvent.click(view.getByRole('button', { name: 'Continue' }));
  await view.findByLabelText('Server address');
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
          agentReady: true,
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

test('first-run waits for shared readiness and never initializes itself', async () => {
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
  let initializations = 0;
  let discoveries = 0;
  const bridge: Bridge = {
    ...mockBridge(),
    native: true,
    firstRunFixture: undefined,
    initializeClientState: async () => {
      initializations++;
      return { state: 'ready' };
    },
    discoverGoProfiles: async () => {
      discoveries++;
      return { installed: false, candidates: [] };
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
        onRefreshWorld: async () => FIXTURE,
        concealSignal: 0,
        agentReady: false,
      }),
    }),
  );
  assert.ok(rendered.getByRole('button', { name: 'Preparing service…' }));
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(initializations, 0);
  assert.equal(discoveries, 0);
});

test('first-run exposes shared agent recovery without discarding its current step', async () => {
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
  let retries = 0;
  const rendered = ui.render(
    createElement(ToastProvider, {
      controller: new ToastController(),
      children: createElement(FirstRunExperience, {
        bridge: { ...mockBridge(), firstRunFixture: undefined },
        world: FIXTURE,
        location: { kind: 'first-run', path: 'own', step: 'address' },
        onNavigate: () => {},
        onRefreshWorld: async () => FIXTURE,
        concealSignal: 0,
        agentReady: false,
        onRetryAgent: async () => {
          retries++;
        },
      }),
    }),
  );
  assert.ok(rendered.getByText('Finish preparing this Mac'));
  assert.equal(rendered.queryByLabelText('Server address'), null);
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Retry setup' }));
  await ui.waitFor(() => assert.equal(retries, 1));
  const saved = JSON.parse(
    window.localStorage.getItem('foks.first-run.v2') ?? '{}',
  ) as { state?: string };
  assert.equal(saved.state, undefined);
});

test('native-shaped account creation is not rewound by the pre-mutation inventory', async () => {
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
  const base = mockBridge(FIXTURE);
  const profile = 'setup-foks-app-4430';
  const hostId = `02${'7'.repeat(64)}`;
  let checks = 0;
  let signups = 0;
  const bridge: Bridge = {
    ...base,
    native: true,
    firstRunFixture: undefined,
    discoverGoProfiles: async () => ({ installed: false, candidates: [] }),
    checkAndAddProfile: async (profileName, probe) => {
      checks++;
      assert.equal(profileName, profile);
      assert.equal(probe, 'foks.app:4430');
      return {
        profile: profileName,
        hostId,
        lookupName: 'foks.app',
        canonicalName: 'foks.app',
        acceptance: 'inserted',
        chain: 1,
        epoch: 1,
      };
    },
    createFirstRunAccount: async (request) => {
      signups++;
      assert.equal(request.profile, profile);
      assert.equal(request.alias, 'native-user');
      return { applied: true };
    },
  };
  const refreshed = {
    ...FIXTURE,
    servers: [
      ...FIXTURE.servers,
      {
        id: profile,
        name: 'foks.app',
        label: null,
        host_id: hostId,
        chain: 1,
        epoch: 1,
        accounts: ['native-user'],
        trust: { status: 'verified' as const },
        compatibility: { status: 'not-required' as const },
        passiveStatus: {
          status: 'available' as const,
          source: 'signed-server-status' as const,
        },
        connectivity: { status: 'unknown' as const },
        capabilities: { chat: false },
        restrictions: [],
      },
    ],
    accounts: [
      ...FIXTURE.accounts,
      {
        store: 'acct:native-user',
        alias: 'native-user',
        username: 'native-user',
        server: profile,
      },
    ],
    profileInventory: [
      ...FIXTURE.profileInventory,
      { profile, accounts: 'complete' as const, teams: 'complete' as const },
    ],
  };
  const view = ui.render(
    createElement(ToastProvider, {
      controller: new ToastController(),
      children: createElement(FirstRunExperience, {
        bridge,
        world: FIXTURE,
        location: { kind: 'first-run', path: 'own', step: 'who' },
        onNavigate: () => {},
        onRefreshWorld: async () => refreshed,
        concealSignal: 0,
        agentReady: true,
      }),
    }),
  );
  await view.findByText('How are you joining?');
  ui.fireEvent.click(view.getByRole('radio', { name: /Set up my own account/ }));
  ui.fireEvent.click(view.getByRole('button', { name: 'Continue' }));
  ui.fireEvent.click(
    view.getByRole('button', { name: 'Use the official FOKS server' }),
  );
  ui.fireEvent.click(view.getByRole('button', { name: 'Use this server' }));
  await view.findByRole('button', { name: 'Details' });
  assert.equal(checks, 1);
  ui.fireEvent.click(view.getByRole('button', { name: 'Continue' }));
  ui.fireEvent.change(view.getByPlaceholderText('yourname'), {
    target: { value: 'native-user' },
  });
  ui.fireEvent.change(view.getByPlaceholderText('Your Mac'), {
    target: { value: 'Native Mac' },
  });
  ui.fireEvent.click(view.getByRole('button', { name: 'Create my account' }));
  await view.findByText('Save recovery phrase');
  assert.equal(signups, 1);
});

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
      throw failure('Refresh the setup status before resuming this operation.');
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
        agentReady: true,
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
        agentReady: true,
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
    window.localStorage.getItem('foks.first-run.v2') ?? '{}',
  ) as { state?: string; group?: unknown };
  assert.equal(checkpoint.state, 'checklist-own');
  assert.equal(checkpoint.group, undefined);
  window.history.replaceState(null, '', '/');
});
