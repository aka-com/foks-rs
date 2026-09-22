import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, StrictMode } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import type { Bridge, CommandError, GoProfileCandidate } from '../src/bridge';
import { identityRetryDelay } from '../src/screens/first-run/use-account-identity';
import { installDom, settle } from './lib/dom-harness';

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
  serverHint: 'foks.app:4430',
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
      await ui.waitFor(() => assert.ok(rendered.getByText('Set up FOKS')));
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
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Continue' }));

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
  for (const name of [/Set up my own account/, /Join an existing team/]) {
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
  await ui.waitFor(() => assert.ok(rendered.getByText('Set up FOKS')));
  assert.equal(
    scans,
    1,
    'Strict Mode shares discovery rather than issuing a duplicate read',
  );
  // A lone usable candidate is preselected under the default radio and its
  // row action reports that selection without offering a second radio level.
  await rendered.findByRole('button', {
    name: /Selected account cli-owner/,
  });
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Continue' }));
  ui.fireEvent.click(
    rendered.getByRole('button', { name: 'Use the official FOKS server' }),
  );
  assert.equal(
    (rendered.getByLabelText('Server address') as HTMLInputElement).value,
    'foks.app:4430',
  );
  ui.fireEvent.click(rendered.getByRole('button', { name: /Continue/ }));
  await ui.waitFor(() => assert.equal(checks.length, 1));
  assert.equal(checks[0][0], candidate.candidateId);
  assert.equal(checks[0][1], candidate.hostId);
  assert.equal(checks[0][3], 'foks.app:4430');
});

test('the start fork selects nested CLI accounts with row actions', async () => {
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
  const second: GoProfileCandidate = {
    ...candidate,
    candidateId: 'second',
    username: 'cli-second',
    deviceId: '03' + '12'.repeat(32),
  };
  const bridge: Bridge = {
    ...mockBridge(),
    native: true,
    discoverGoProfiles: async () => ({
      installed: true,
      candidates: [candidate, second],
    }),
  };
  const view = ui.render(
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
  );
  await view.findByRole('heading', { name: 'Set up FOKS' });
  // The lead is the same sentence however many candidates were found.
  assert.ok(view.getByText(/A FOKS CLI account was found on this Mac/));
  const fork = view.getByRole('radiogroup', { name: 'How to start' });
  const existing = ui.within(fork).getByRole('radio', {
    name: /^Use an existing account/,
  });
  const fresh = ui.within(fork).getByRole('radio', {
    name: /^Create a new account/,
  });
  const primary = () =>
    view.getByRole('button', { name: 'Continue' }) as HTMLButtonElement;
  // The existing account is the default; with two candidates none is chosen
  // yet, so Continue waits.
  assert.equal(existing.getAttribute('aria-checked'), 'true');
  assert.equal(fresh.getAttribute('aria-checked'), 'false');
  const accounts = view.getByRole('group', { name: 'FOKS accounts' });
  assert.ok(fork.contains(accounts), 'the list is nested in the fork');
  assert.ok(accounts.classList.contains('nested'));
  assert.equal(ui.within(accounts).queryAllByRole('radio').length, 0);
  assert.equal(
    ui.within(accounts).getAllByRole('button', { name: /^Select account/ })
      .length,
    2,
  );
  // Each row is a two-line entry: the name, then server, role and storage.
  const texts = [...accounts.querySelectorAll('.go-account-text')];
  assert.equal(texts.length, 2);
  assert.equal(texts[0].querySelector('b')?.textContent, 'cli-owner');
  assert.equal(
    texts[0].querySelector('small')?.textContent,
    'foks.app · Account owner · Keychain storage',
  );
  assert.equal(primary().disabled, true);
  assert.equal(
    view.queryByRole('button', { name: 'Create a new account' }),
    null,
  );
  // The new-account radio hides the list and is enough on its own.
  ui.fireEvent.click(fresh);
  assert.equal(fresh.getAttribute('aria-checked'), 'true');
  assert.equal(view.queryByRole('group', { name: 'FOKS accounts' }), null);
  assert.equal(primary().disabled, false);
  // Back to the existing account: the list returns, still unchosen.
  ui.fireEvent.click(existing);
  assert.equal(primary().disabled, true);
  ui.fireEvent.click(
    view.getByRole('button', { name: /Select account cli-second/ }),
  );
  assert.equal(primary().disabled, false);
  const selected = view.getByRole('button', {
    name: /Selected account cli-second/,
  }) as HTMLButtonElement;
  assert.equal(selected.disabled, true);
  assert.equal(selected.textContent, 'Selected');
  ui.fireEvent.click(primary());
  // Continue with the chosen account leaves for the server step.
  assert.ok(view.getByRole('button', { name: 'Use the official FOKS server' }));
  assert.equal(view.queryByRole('heading', { name: 'Set up FOKS' }), null);
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
  assert.equal(rendered.queryByText('CLI accounts on this device'), null);
  const continueButton = rendered.getByRole('button', {
    name: 'Continue',
  }) as HTMLButtonElement;
  assert.equal(continueButton.disabled, true);
  ui.fireEvent.click(rendered.getByRole('radio', { name: /cli-owner/ }));
  assert.equal(
    (rendered.getByRole('radio', { name: /cli-owner/ }) as HTMLInputElement)
      .checked,
    true,
  );
  assert.equal(rendered.queryByLabelText('Server address'), null);
  assert.equal(continueButton.disabled, false);
  assert.ok(continueButton.classList.contains('primary'));
  ui.fireEvent.click(continueButton);
  ui.fireEvent.click(
    rendered.getByRole('button', { name: 'Use official FOKS server' }),
  );
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Import' }));
  assert.equal(
    (rendered.getByLabelText('Server address') as HTMLInputElement).disabled,
    true,
  );
  assert.equal(
    (
      rendered.getByRole('button', {
        name: 'Back',
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
  await settle();
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
  const profile = 'foks-app';
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
        profileName: profile,
        displayLabel: null,
        configuredEndpoint: 'foks.app',
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
        services: { chat: false },
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
  ui.fireEvent.click(view.getByRole('button', { name: 'Continue' }));
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
  await view.findByRole('button', { name: /Selected account cli-owner/ });
  // The "Create a new account" radio hides the account list; Continue then
  // leads to "How are you joining?", whose Back returns to the fork.
  ui.fireEvent.click(view.getByRole('radio', { name: /Create a new account/ }));
  assert.equal(view.queryByRole('group', { name: 'FOKS accounts' }), null);
  ui.fireEvent.click(view.getByRole('button', { name: 'Continue' }));
  assert.ok(view.getByRole('heading', { name: 'How are you joining?' }));
  assert.ok(view.queryByText(/Already using FOKS/) === null);
  assert.ok(view.queryByText(/You’ll need/) === null);
  ui.fireEvent.click(view.getByRole('button', { name: 'Back' }));
  assert.ok(view.getByRole('heading', { name: 'Set up FOKS' }));
  ui.fireEvent.click(
    view.getByRole('radio', { name: /^Use an existing account/ }),
  );
  assert.ok(view.getByRole('button', { name: /Selected account cli-owner/ }));
  ui.fireEvent.click(view.getByRole('button', { name: 'Continue' }));
  ui.fireEvent.click(
    view.getByRole('button', { name: 'Use the official FOKS server' }),
  );
  ui.fireEvent.click(view.getByRole('button', { name: 'Continue' }));
  await view.findByRole('button', { name: 'Details' });
  assert.ok(view.queryByText('Pinned on this device') === null);
  const addressField = view.getByLabelText('Server address');
  addressField.focus();
  ui.fireEvent.change(addressField, { target: { value: 'changed.example' } });
  assert.equal(document.activeElement, view.getByLabelText('Server address'));
  assert.ok(view.queryByRole('button', { name: 'Details' }) === null);
  ui.fireEvent.click(view.getByRole('button', { name: 'Continue' }));
  await view.findByRole('button', { name: 'Details' });
  ui.fireEvent.click(view.getByRole('button', { name: 'Continue' }));
  // The sign-in method is a second radio group, at the same width as the
  // setup-method group; with three ways in, none is chosen yet, so the
  // account rows and the foot's action wait for the choice.
  const methods = view.getByRole('radiogroup', { name: 'Sign-in method' });
  assert.deepEqual(
    ui
      .within(methods)
      .getAllByRole('radio')
      .map((el) => el.textContent),
    [
      'Use the FOKS CLI to link this as a new devicePair the desktop app with foks --simple-ui key assist in Terminal.',
      'Import this device’s FOKS CLI credentialsCopy your device credentials from the FOKS CLI. This may require a Keychain prompt.',
      'Recover with recovery phraseEnter your recovery phrase to restore full access on this device.',
    ],
  );
  const pairMethod = ui.within(methods).getByRole('radio', {
    name: /Use the FOKS CLI to link this as a new device/,
  });
  assert.equal(
    pairMethod.querySelector('strong')?.textContent,
    'foks --simple-ui key assist',
  );
  assert.equal(pairMethod.querySelector('code'), null);
  assert.deepEqual(
    [...view.container.querySelectorAll('.sec.step')].map(
      (el) => el.textContent,
    ),
    ['1Setup method', '2Sign in method'],
  );
  assert.equal(view.queryByLabelText('Your name'), null);
  assert.ok(
    (view.getByRole('button', { name: 'Continue' }) as HTMLButtonElement)
      .disabled,
  );
  ui.fireEvent.click(
    view.getByRole('radio', { name: /Import this device’s FOKS CLI/ }),
  );
  assert.equal(
    view.container.querySelectorAll('.sec.step')[2]?.textContent,
    '3Account and device',
  );
  assert.ok(view.getByLabelText('Your name'));
  assert.equal(view.queryByLabelText('Recovery phrase'), null);
  assert.ok(
    view.getByText(
      'Both apps will share the same device credentials. Revoking the device in either client will disable both.',
    ),
  );
  ui.fireEvent.click(view.getByRole('button', { name: 'Import credentials' }));
  await view.findByText('Copy failed');
  ui.fireEvent.click(
    view.getByRole('radio', { name: /Recover with recovery phrase/ }),
  );
  assert.equal(view.queryByText('Copy failed'), null);
  assert.equal(
    view.queryByRole('button', { name: 'Import credentials' }),
    null,
  );
  ui.fireEvent.change(view.getByLabelText('Recovery phrase'), {
    target: { value: 'one two three' },
  });
  ui.fireEvent.click(view.getByRole('button', { name: 'Recover' }));
  await view.findByText('Recovery failed');
  // Choosing a CLI account at the start fixes the account setup path. The
  // creation alternatives stay visible for context but cannot replace it.
  const create = view.getByRole('radio', {
    name: /Create a new account/,
  }) as HTMLButtonElement;
  const organization = view.getByRole('radio', {
    name: /Sign up with your organization/,
  }) as HTMLButtonElement;
  assert.equal(create.disabled, true);
  assert.equal(organization.disabled, true);
  assert.equal(create.classList.contains('off'), true);
  assert.equal(organization.classList.contains('off'), true);
  ui.fireEvent.click(create);
  assert.ok(view.getByRole('radiogroup', { name: 'Sign-in method' }));
  ui.fireEvent.click(
    view.getByRole('radio', { name: /Use the FOKS CLI to link/ }),
  );
  assert.ok(view.getByLabelText('Pairing phrase'));
  assert.ok(
    (view.getByRole('button', { name: 'Accept pairing' }) as HTMLButtonElement)
      .disabled,
  );
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
    ['Recovery phrase', 'Passphrase'],
  );
  assert.ok(view.queryByText(/hardware key/i) === null);
  assert.ok(view.queryByText('Create a group') === null);
  ui.fireEvent.click(view.getByRole('button', { name: 'Continue' }));
  await view.findByRole('button', { name: 'Continue to my vault' });
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
      transitionFirstRun(checked, {
        type: 'select-account-method',
        method: 'recover',
      }),
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
  await rendered.findByText('Recover with recovery phrase');
  await rendered.findByText('Import this device’s FOKS CLI credentials');
  rendered.getByText('Use the FOKS CLI to link this as a new device');
  assert.equal(scans, 1);
  // The account rows follow the method choice.
  assert.equal(rendered.queryByLabelText('Your name'), null);
  ui.fireEvent.click(
    rendered.getByRole('radio', { name: /Recover with recovery phrase/ }),
  );
  assert.equal(
    (rendered.getByLabelText('Your name') as HTMLInputElement).value,
    'cli-owner',
    'the CLI username is suggested as the local alias, as the chooser does',
  );
});

test('identity loading waits out a native mutation instead of failing setup', async (t) => {
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
        profileName: profile.profile,
        configuredEndpoint: profile.canonicalName,
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
  // Fake the browser clock, leaving testing-library's own timers real.
  let now = 0;
  let nextTimer = 0;
  const timers = new Map<number, { due: number; run(): void }>();
  t.mock.method(window, 'setTimeout', (callback: () => void, delay = 0) => {
    const id = ++nextTimer;
    timers.set(id, { due: now + delay, run: callback });
    return id;
  });
  t.mock.method(window, 'clearTimeout', (id: number) => timers.delete(id));
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
  // The retry schedule doubles from two seconds to a ceiling, so the second
  // wait is longer than the first. Drive it by the delay the probe actually
  // scheduled rather than assuming a flat cadence.
  for (let attempt = 1; attempt <= 2; attempt++) {
    assert.equal(refreshes, attempt);
    const due = [...timers.values()]
      .map((timer) => timer.due)
      .filter((at) => at > now)
      .sort((a, b) => a - b)[0];
    assert.ok(due !== undefined, 'a busy refresh schedules another attempt');
    assert.equal(
      due - now,
      identityRetryDelay(attempt - 1),
      'the busy refresh did not use the probe’s own backoff',
    );
    await ui.act(async () => {
      now = due;
      for (const [id, timer] of [...timers]) {
        if (timer.due <= now && timers.delete(id)) timer.run();
      }
    });
  }
  await ui.waitFor(
    () => {
      const saved = decodeFirstRunCheckpoint(
        window.localStorage.getItem(FIRST_RUN_CHECKPOINT_KEY),
      );
      assert.equal(saved?.state, 'protect');
      assert.equal(saved?.account?.alias, 'cli-owner');
    },
    { timeout: 1_000 },
  );
  assert.equal(refreshes, 3);
  assert.equal(rendered.queryByRole('alert'), null);
});

test('a server added after unmount is listed on Accounts and pairs without another add', async () => {
  const { AccountSection } = (await vite.ssrLoadModule(
    '/src/screens/account-section.tsx',
  )) as typeof import('../src/screens/account-section');
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
        profileName: 'cli-local',
        displayLabel: 'CLI server',
        host_id: candidate.hostId,
        accounts: [],
      },
    ],
    profileInventory: [
      ...FIXTURE.profileInventory,
      {
        profile: 'cli-local',
        accounts: 'complete' as const,
        teams: 'complete' as const,
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
  ui.fireEvent.click(adding.getByRole('button', { name: 'Continue' }));
  ui.fireEvent.click(
    adding.getByRole('button', { name: 'Use official FOKS server' }),
  );
  ui.fireEvent.click(adding.getByRole('button', { name: 'Import' }));
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
      createElement(AccountSection, {
        snapshot,
        bridge,
        location: { kind: 'settings', section: 'account' },
        panel: { id: 'panel', labelledBy: 'tab' },
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
  // The host is the row's value, not its label: the label column is a fixed
  // width and a hostname overran it, so the row reads "Server" and states the
  // host and its pairing state together.
  // This Mac holds another unpaired server as well, so the row is the one
  // that names this server rather than the only one in that state.
  const row = [...accounts.container.querySelectorAll<HTMLElement>('.fr')].find(
    (candidate) => candidate.textContent?.includes('CLI server'),
  );
  assert.ok(row);
  assert.ok(ui.within(row).getByText('Server'));
  assert.ok(
    ui.within(row).getByText('Connected, not paired').classList.contains('dim'),
  );
  ui.fireEvent.click(ui.within(row).getByRole('button', { name: 'Pair' }));
  ui.fireEvent.click(await accounts.findByRole('radio', { name: /cli-owner/ }));
  ui.fireEvent.click(accounts.getByRole('button', { name: 'Continue' }));
  assert.ok(accounts.getByRole('button', { name: 'Pair this device' }));
  assert.ok(accounts.getByRole('button', { name: 'Resume pairing' }));
  assert.equal(accounts.queryByRole('button', { name: 'Import' }), null);
  assert.equal(adds, 1);
});

test('CLI pairing resumes without entering a new device name', async () => {
  const { GoProfileConnectSheet } = (await vite.ssrLoadModule(
    '/src/screens/go-profile-connect.tsx',
  )) as typeof import('../src/screens/go-profile-connect');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const { OverlayProvider } = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives');
  const calls: string[][] = [];
  const bridge: Bridge = {
    ...mockBridge(),
    discoverGoProfiles: async () => ({
      installed: true,
      candidates: [candidate],
    }),
    resumeGoProfilePairing: async (...args) => {
      calls.push(args);
      throw new Error('Saved pairing is still pending');
    },
  };
  const rendered = ui.render(
    createElement(OverlayProvider, {
      backgroundRef: { current: null },
      portalRoot: document.getElementById('overlays')!,
      children: createElement(GoProfileConnectSheet, {
        bridge,
        existingProfile: { profileName: 'local', host_id: candidate.hostId },
        onClose: () => {},
        onConnected: async () => {},
        onError: () => {},
      }),
    }),
  );
  ui.fireEvent.click(await rendered.findByRole('radio', { name: /cli-owner/ }));
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Continue' }));
  assert.equal(
    (rendered.getByLabelText('Device name') as HTMLInputElement).value,
    '',
  );
  assert.equal(
    (
      rendered.getByRole('button', {
        name: 'Pair this device',
      }) as HTMLButtonElement
    ).disabled,
    true,
  );
  const resume = rendered.getByRole('button', {
    name: 'Resume pairing',
  }) as HTMLButtonElement;
  assert.equal(resume.disabled, false);
  ui.fireEvent.click(resume);
  await ui.waitFor(() =>
    assert.deepEqual(calls, [['candidate', 'local', 'cli-owner']]),
  );
});
