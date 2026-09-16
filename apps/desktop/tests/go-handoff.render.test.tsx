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
  const checkpoint = encodeFirstRunCheckpoint(initialFirstRun('own', 'who'));
  window.localStorage.setItem(FIRST_RUN_CHECKPOINT_KEY, checkpoint);
  const bridge = {
    ...mockBridge(snapshot),
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
          snapshot,
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
        assert.ok(rendered.getByText('Finish preparing this device')),
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
      assert.ok(rendered.getByText('Finish preparing this device'));
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
      assert.ok(rendered.getByText('Finish preparing this device')),
    );
    assert.equal(
      rendered.queryByText(/Could not check existing CLI profiles/),
      null,
    );
  });
}

test('first-run keeps a newly verified server across shell navigation', async () => {
  const harness = await discoveryRecoveryHarness();
  let checks = 0;
  const rendered = harness.render({
    discoverGoProfiles: async () => ({ installed: false, candidates: [] }),
    checkAndAddProfile: async (profile, probe) => {
      checks++;
      return {
        profile,
        hostId: `02${'7'.repeat(64)}`,
        lookupName: probe,
        canonicalName: 'foks.app',
        acceptance: 'inserted',
        chain: 1,
        epoch: 1,
      };
    },
  });
  await rendered.findByText('How are you joining?');
  ui.fireEvent.click(
    rendered.getByRole('radio', { name: /Set up my own account/ }),
  );
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Continue' }));
  ui.fireEvent.click(
    rendered.getByRole('button', { name: 'Use the official FOKS server' }),
  );
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Use this server' }));

  await ui.waitFor(() => assert.equal(checks, 1));
  await rendered.findByRole('button', { name: 'Details' });
});

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
      (view.getByRole('button', { name: 'Continue' }) as HTMLButtonElement)
        .disabled,
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
          snapshot: FIXTURE,
          location: { kind: 'first-run', path: 'own', step: 'who' },
          onNavigate: () => {},
          onRefreshSnapshot: async () => FIXTURE,
          concealSignal: 0,
          agentReady: true,
        }),
      }),
    ),
  );
  await ui.waitFor(() =>
    assert.ok(rendered.getByText('Select an FOKS account')),
  );
  assert.equal(
    scans,
    1,
    'Strict Mode shares discovery rather than issuing a duplicate read',
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
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Add server' }));
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
  assert.equal(rendered.queryByText('Pair this device'), null);
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
        snapshot: FIXTURE,
        location: { kind: 'first-run', path: 'own', step: 'who' },
        onNavigate: () => {},
        automaticEntry: true,
        onRefreshSnapshot: async () => FIXTURE,
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
        snapshot: FIXTURE,
        location: { kind: 'first-run', path: 'own', step: 'address' },
        onNavigate: () => {},
        onRefreshSnapshot: async () => FIXTURE,
        concealSignal: 0,
        agentReady: false,
        onRetryAgent: async () => {
          retries++;
        },
      }),
    }),
  );
  assert.ok(rendered.getByText('Finish preparing this device'));
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
        configuredProbe: 'foks.app',
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
        snapshot: FIXTURE,
        location: { kind: 'first-run', path: 'own', step: 'who' },
        onNavigate: () => {},
        onRefreshSnapshot: async () => refreshed,
        concealSignal: 0,
        agentReady: true,
      }),
    }),
  );
  await view.findByText('How are you joining?');
  ui.fireEvent.click(
    view.getByRole('radio', { name: /Set up my own account/ }),
  );
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
  ui.fireEvent.change(view.getByPlaceholderText('Your device'), {
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
        snapshot: FIXTURE,
        location: { kind: 'first-run', path: 'own', step: 'who' },
        concealSignal: 0,
        agentReady: true,
        onNavigate: (location: unknown) => {
          navigations.push(location);
        },
        onRefreshSnapshot: async () => FIXTURE,
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
  assert.ok(view.queryByText('Pinned on this device') === null);
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
    'Import this device’s FOKS CLI credentials',
    'Use the CLI to approve this as a new device',
  ]);
  ui.fireEvent.click(view.getByRole('button', { name: 'Import credentials' }));
  const copyError = await view.findByText('Copy failed');
  assert.equal(
    copyError.closest('.pcard')?.querySelector('h3')?.textContent,
    'Import this device’s FOKS CLI credentials',
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
  // Both choices live on Set up your account: switching the radio swaps what is
  // drawn under it without leaving the page.
  ui.fireEvent.click(view.getByRole('radio', { name: /Create a new account/ }));
  assert.ok(view.getByPlaceholderText('yourname'));
  assert.ok(view.getByPlaceholderText('Your device'));
  assert.equal(view.queryByText('Recover with your backup phrase'), null);
  ui.fireEvent.click(
    view.getByRole('radio', { name: /Sign in to an existing account/ }),
  );
  assert.ok(view.getByText('Recover with your backup phrase'));
  assert.ok(view.getByRole('heading', { name: 'Set up your account' }));
  ui.fireEvent.click(view.getByRole('button', { name: 'Resume pairing' }));
  await view.findByRole('heading', { name: 'Check account setup' });
  assert.equal(view.queryByRole('button', { name: 'Recover' }), null);
  assert.equal(
    view.queryByRole('button', { name: 'Import credentials' }),
    null,
  );
  ui.fireEvent.click(view.getByRole('button', { name: 'Finish later' }));
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
        snapshot: FIXTURE,
        location: { kind: 'first-run', path: 'own', step: 'protect' },
        onNavigate: () => {},
        onRefreshSnapshot: async () => FIXTURE,
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

test('resumed sign-in step rediscovers the CLI profile for the verified server', async () => {
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
  const {
    initialFirstRun,
    transitionFirstRun,
    encodeFirstRunCheckpoint,
    FIRST_RUN_CHECKPOINT_KEY,
  } = (await vite.ssrLoadModule(
    '/src/first-run-state.ts',
  )) as typeof import('../src/first-run-state');
  // A checkpoint saved after server verification, restarted at the sign-in
  // step without passing through the joining step's CLI chooser.
  const chosen = transitionFirstRun(initialFirstRun('own'), {
    type: 'choose',
    path: 'own',
    returning: true,
  });
  const checked = transitionFirstRun(chosen, {
    type: 'profile-checked',
    address: 'foks.app:4430',
    profile: {
      profile: 'setup-foks-app-4430',
      acceptance: 'inserted',
      lookupName: 'foks.app:4430',
      canonicalName: 'foks.app',
      hostId: candidate.hostId,
      chain: 1,
      epoch: 1,
    },
  });
  window.localStorage.setItem(
    FIRST_RUN_CHECKPOINT_KEY,
    encodeFirstRunCheckpoint(
      transitionFirstRun(checked, { type: 'go', state: 'existing' }),
    ),
  );
  const snapshot = {
    ...FIXTURE,
    profileInventoryStatus: 'unavailable' as const,
  };
  let scans = 0;
  const bridge: Bridge = {
    ...mockBridge(),
    native: true,
    firstRunFixture: undefined,
    discoverGoProfiles: async () => {
      scans++;
      return {
        installed: true,
        candidates: [
          candidate,
          // Another server's profile must not be selected.
          {
            ...candidate,
            candidateId: 'elsewhere',
            username: 'other-owner',
            hostId: '02' + '99'.repeat(32),
          },
        ],
      };
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
          snapshot,
          location: { kind: 'first-run', path: 'own', step: 'existing' },
          onNavigate: () => {},
          onRefreshSnapshot: async () => snapshot,
          concealSignal: 0,
          agentReady: true,
        }),
      }),
    ),
  );
  await rendered.findByText('Recover with your backup phrase');
  await rendered.findByText('Import this device’s FOKS CLI credentials');
  rendered.getByText('Use the CLI to approve this as a new device');
  assert.equal(scans, 1);
  assert.equal(
    (rendered.getByLabelText('Copied account alias') as HTMLInputElement).value,
    'cli-owner',
    'the CLI username is suggested as the local alias, as the chooser does',
  );
});

test('identity loading waits out a native mutation instead of failing setup', async () => {
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
  const {
    initialFirstRun,
    transitionFirstRun,
    encodeFirstRunCheckpoint,
    decodeFirstRunCheckpoint,
    FIRST_RUN_CHECKPOINT_KEY,
  } = (await vite.ssrLoadModule(
    '/src/first-run-state.ts',
  )) as typeof import('../src/first-run-state');
  const profile = {
    profile: 'setup-foks-app-4430',
    acceptance: 'inserted' as const,
    lookupName: 'foks.app:4430',
    canonicalName: 'foks.app',
    hostId: candidate.hostId,
    chain: 1,
    epoch: 1,
  };
  let checkpoint = transitionFirstRun(initialFirstRun('own'), {
    type: 'choose',
    path: 'own',
    returning: true,
  });
  checkpoint = transitionFirstRun(checkpoint, {
    type: 'profile-checked',
    address: 'foks.app:4430',
    profile,
  });
  checkpoint = transitionFirstRun(checkpoint, {
    type: 'account-provisioned',
    alias: 'cli-owner',
    deviceName: 'Test Mac',
  });
  assert.equal(checkpoint.state, 'identity-pending');
  window.localStorage.setItem(
    FIRST_RUN_CHECKPOINT_KEY,
    encodeFirstRunCheckpoint(checkpoint),
  );
  // The snapshot before the refresh has no usable inventory, so the identity
  // stays pending until a refresh succeeds.
  const pending = {
    ...FIXTURE,
    profileInventoryStatus: 'unavailable' as const,
  };
  const connected = {
    ...FIXTURE,
    servers: [
      {
        ...FIXTURE.servers[0],
        id: profile.profile,
        name: profile.canonicalName,
        host_id: profile.hostId,
        accounts: ['cli-owner'],
      },
    ],
    accounts: [
      {
        store: 'cli-owner-store',
        alias: 'cli-owner',
        username: 'cli-owner',
        server: profile.profile,
      },
    ],
    profileInventory: [
      {
        profile: profile.profile,
        accounts: 'complete' as const,
        teams: 'complete' as const,
      },
    ],
    catalogProfiles: [profile.profile],
    profileInventoryStatus: 'complete' as const,
  };
  let refreshes = 0;
  const rendered = ui.render(
    createElement(ToastProvider, {
      controller: new ToastController(),
      children: createElement(FirstRunExperience, {
        bridge: { ...mockBridge(), native: true, firstRunFixture: undefined },
        snapshot: pending,
        location: { kind: 'first-run', path: 'own', step: 'identity-pending' },
        onNavigate: () => {},
        onRefreshSnapshot: async () => {
          refreshes++;
          // A native mutation still holds the catalog for the first two reads.
          if (refreshes <= 2)
            throw {
              code: 'mutation-in-flight',
              message:
                'Wait for the current operation to finish before refreshing.',
              retryable: false,
              ambiguous: false,
              fatal: false,
            };
          return connected;
        },
        concealSignal: 0,
        agentReady: true,
      }),
    }),
  );
  await rendered.findByText('Waiting for the current operation to finish…');
  assert.equal(rendered.queryByRole('alert'), null);
  assert.equal(
    (
      rendered.getByRole('button', {
        name: /Loading account details/,
      }) as HTMLButtonElement
    ).disabled,
    true,
  );
  await ui.waitFor(
    () => {
      const saved = decodeFirstRunCheckpoint(
        window.localStorage.getItem(FIRST_RUN_CHECKPOINT_KEY),
      );
      assert.equal(saved?.state, 'protect');
      assert.equal(saved?.account?.alias, 'cli-owner');
    },
    { timeout: 8_000 },
  );
  assert.equal(refreshes, 3);
  assert.equal(rendered.queryByRole('alert'), null);
});

test('a server added after unmount is listed on Accounts and pairs without another add', async () => {
  const { PeopleScreen } = (await vite.ssrLoadModule(
    '/src/screens/people-screen.tsx',
  )) as typeof import('../src/screens/people-screen');
  const { GoProfileConnectSheet } = (await vite.ssrLoadModule(
    '/src/screens/go-profile-connect.tsx',
  )) as typeof import('../src/screens/go-profile-connect');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const { OverlayProvider } = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives');
  const { ToastProvider, ToastController } = (await vite.ssrLoadModule(
    '/kit/toasts.tsx',
  )) as typeof import('../kit/toasts');
  const snapshot = {
    ...FIXTURE,
    servers: [
      ...FIXTURE.servers,
      {
        ...FIXTURE.servers[0],
        id: 'cli-local',
        name: 'CLI server',
        label: null,
        host_id: candidate.hostId,
        accounts: [],
      },
    ],
  };
  let resolve!: (
    value: Awaited<ReturnType<Bridge['checkAndAddGoProfile']>>,
  ) => void;
  let adds = 0;
  const added: string[] = [];
  const bridge: Bridge = {
    ...mockBridge(snapshot),
    discoverGoProfiles: async () => ({
      installed: true,
      candidates: [candidate],
    }),
    checkAndAddGoProfile: () => {
      adds++;
      return new Promise((done) => {
        resolve = done;
      });
    },
  };
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  const wrap = (children: import('react').ReactNode) =>
    createElement(OverlayProvider, {
      backgroundRef: { current: null },
      portalRoot,
      children: createElement(ToastProvider, {
        controller: new ToastController(),
        portalRoot,
        children,
      }),
    });
  const adding = ui.render(
    wrap(
      createElement(GoProfileConnectSheet, {
        bridge,
        onClose: () => {},
        onConnected: async () => {},
        onError: (error) => {
          throw error;
        },
        onAdded: async (profile) => {
          added.push(profile);
        },
      }),
    ),
  );
  ui.fireEvent.click(await adding.findByRole('radio', { name: /cli-owner/ }));
  ui.fireEvent.click(
    adding.getByRole('button', { name: 'Use official FOKS server' }),
  );
  ui.fireEvent.click(adding.getByRole('button', { name: 'Add server' }));
  adding.unmount();
  await ui.act(async () => {
    resolve({
      profile: 'cli-local',
      acceptance: 'inserted',
      lookupName: 'foks.app',
      canonicalName: 'foks.app',
      hostId: candidate.hostId,
      chain: 1,
      epoch: 1,
    });
  });
  assert.deepEqual(added, ['cli-local']);
  const accounts = ui.render(
    wrap(
      createElement(PeopleScreen, {
        snapshot,
        bridge,
        location: { kind: 'people' },
        onNavigate: () => {},
        onRefresh: async () => {},
        onRefreshSnapshot: async () => snapshot,
        onError: (error) => {
          throw error;
        },
        onMutationError: async (error) => {
          throw error;
        },
      }),
    ),
  );
  const row = accounts.getByText('CLI server').parentElement;
  assert.ok(row);
  assert.ok(ui.within(row).getByText('Connected, not yet paired'));
  ui.fireEvent.click(ui.within(row).getByRole('button', { name: 'Pair' }));
  ui.fireEvent.click(await accounts.findByRole('radio', { name: /cli-owner/ }));
  assert.ok(accounts.getByRole('button', { name: 'Pair this device' }));
  assert.ok(accounts.getByRole('button', { name: 'Resume pairing' }));
  assert.equal(accounts.queryByRole('button', { name: 'Add server' }), null);
  assert.equal(adds, 1);
});
