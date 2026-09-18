import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import type { Bridge } from '../src/bridge';
import type { FirstRunExperienceProps } from '../src/screens/first-run-screen';
import type { FirstRunCheckpoint } from '../src/first-run-state';
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

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

async function harness() {
  const { FirstRunExperience } = (await vite.ssrLoadModule(
    '/src/screens/first-run-screen.tsx',
  )) as typeof import('../src/screens/first-run-screen');
  const { ToastProvider, ToastController } = (await vite.ssrLoadModule(
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
  const state = (await vite.ssrLoadModule(
    '/src/first-run-state.ts',
  )) as typeof import('../src/first-run-state');
  const profile = {
    profile: 'personal',
    hostId: `02${'2'.repeat(64)}`,
    lookupName: 'localhost',
    canonicalName: 'localhost',
    acceptance: 'inserted' as const,
    chain: 1,
    epoch: 1,
  };
  const complete = {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server) =>
      server.id === 'personal'
        ? { ...server, host_id: profile.hostId }
        : server,
    ),
  };
  const unknown = {
    ...complete,
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
  const checkpoint: FirstRunCheckpoint = {
    ...state.initialFirstRun('own', 'protect'),
    managedLocal: true,
    serverAddress: 'localhost:4430',
    profile,
    account: { alias: 'personal', username: 'satoshi', deviceName: 'Mac' },
  };
  const bridge: Bridge = {
    ...mockBridge(complete),
    native: true,
    firstRunFixture: undefined,
  };
  const controller = new ToastController();
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  const element = (props: FirstRunExperienceProps) =>
    createElement(OverlayProvider, {
      backgroundRef: { current: null },
      portalRoot,
      children: createElement(ToastProvider, {
        controller,
        children: createElement(FirstRunExperience, props),
      }),
    });
  return {
    ...state,
    checkpoint,
    bridge,
    complete,
    unknown,
    saved: () =>
      state.decodeFirstRunCheckpoint(
        window.localStorage.getItem(state.FIRST_RUN_CHECKPOINT_KEY),
      ),
    render(
      saved = checkpoint,
      overrides: Partial<FirstRunExperienceProps> = {},
    ) {
      window.localStorage.setItem(
        state.FIRST_RUN_CHECKPOINT_KEY,
        state.encodeFirstRunCheckpoint(saved),
      );
      let props: FirstRunExperienceProps = {
        bridge,
        snapshot: unknown,
        location: { kind: 'first-run', path: 'own', step: saved.state },
        onNavigate: () => {},
        onRefreshSnapshot: async () => unknown,
        concealSignal: 0,
        agentReady: true,
        ...overrides,
      };
      const view = ui.render(element(props));
      return {
        view,
        rerender(update: Partial<FirstRunExperienceProps>) {
          props = { ...props, ...update };
          view.rerender(element(props));
        },
      };
    },
  };
}

test('leaving server entry while a cosmetic label write settles cannot accept the old check', async () => {
  const h = await harness();
  const label = deferred<Awaited<ReturnType<Bridge['setServerLabel']>>>();
  let labels = 0;
  const rendered = h.render(
    { ...h.initialFirstRun('own', 'address'), serverAddress: 'localhost:4430' },
    {
      bridge: {
        ...h.bridge,
        checkAndAddProfile: async () => ({
          ...h.checkpoint.profile!,
          profile: 'localhost',
        }),
        setServerLabel: () => {
          labels++;
          return label.promise;
        },
      },
    },
  );
  ui.fireEvent.click(rendered.view.getByRole('button', { name: 'Continue' }));
  await ui.waitFor(() => assert.equal(labels, 1));
  rendered.rerender({
    location: { kind: 'first-run', path: 'own', step: 'who' },
  });
  await ui.act(async () => {
    label.resolve({ profile: 'localhost', label: 'localhost', changed: true });
  });
  assert.equal(h.saved()?.state, 'who');
  assert.equal(h.saved()?.profile, undefined);
});

test('completed local setup explains unavailable inventory and offers a retry through failure and success', async () => {
  const h = await harness();
  let refreshes = 0;
  const saved = {
    ...h.checkpoint,
    state: 'local-done' as const,
    backupCommitted: true,
  };
  const rendered = h.render(saved, {
    onRefreshSnapshot: async () => {
      refreshes++;
      if (refreshes === 1) throw new Error('Inventory unavailable');
      rendered.rerender({ snapshot: h.complete });
      return h.complete;
    },
  });
  assert.ok(rendered.view.getByText('Setup is complete'));
  assert.equal(rendered.view.queryByText('Your Personal vault is ready'), null);
  ui.fireEvent.click(
    rendered.view.getByRole('button', {
      name: 'Retry loading Personal vault',
    }),
  );
  await rendered.view.findByText('Inventory unavailable');
  assert.equal(h.saved()?.state, 'local-done');
  ui.fireEvent.click(
    rendered.view.getByRole('button', {
      name: 'Retry loading Personal vault',
    }),
  );
  await rendered.view.findByRole('button', { name: 'Open Personal' });
  assert.equal(refreshes, 2);
  assert.equal(h.saved()?.account?.alias, 'personal');
});

test('uncertain signup requires explicit pending-operation resume even when the alias already exists', async () => {
  const h = await harness();
  let creates = 0;
  let resumes = 0;
  const bridge: Bridge = {
    ...h.bridge,
    createFirstRunAccount: async () => {
      creates++;
      throw {
        code: 'ambiguous',
        message: 'Lost reply',
        ambiguous: true,
        retryable: false,
        fatal: false,
      };
    },
    listPendingOperations: async () => [
      { kind: 'account-signup', alias: 'personal' },
    ],
    resumeFirstRunAccount: async () => {
      resumes++;
      return { applied: true };
    },
  };
  // The pending row becomes available only after the first attempt.
  bridge.listPendingOperations = async () =>
    creates ? [{ kind: 'account-signup', alias: 'personal' }] : [];
  const rendered = h.render(
    { ...h.checkpoint, account: undefined, state: 'account' },
    {
      bridge,
      snapshot: h.complete,
      onRefreshSnapshot: async () => h.complete,
    },
  );
  ui.fireEvent.change(rendered.view.getByPlaceholderText('yourname'), {
    target: { value: 'personal' },
  });
  ui.fireEvent.click(
    rendered.view.getByRole('button', { name: 'Create my account' }),
  );
  await rendered.view.findByRole('button', { name: 'Resume account setup' });
  assert.equal(h.saved()?.state, 'operation-pending');
  assert.equal(resumes, 0);
  ui.fireEvent.click(
    rendered.view.getByRole('button', { name: 'Resume account setup' }),
  );
  await rendered.view.findByRole('button', { name: 'Show recovery phrase' });
  assert.equal(creates, 1);
  assert.equal(resumes, 1);
});

test('slow CLI discovery can be skipped and late results do not reopen the chooser', async () => {
  const h = await harness();
  const discovery =
    deferred<Awaited<ReturnType<Bridge['discoverGoProfiles']>>>();
  let slow!: () => void;
  const original = window.setTimeout.bind(window);
  window.setTimeout = ((callback: () => void, delay?: number) => {
    if (delay === 8000) {
      slow = callback;
      return 0;
    }
    return original(callback, delay);
  }) as typeof window.setTimeout;
  try {
    const rendered = h.render(h.initialFirstRun('own', 'who'), {
      bridge: { ...h.bridge, discoverGoProfiles: () => discovery.promise },
    });
    await ui.act(async () => slow());
    assert.ok(rendered.view.getByText(/taking longer than expected/));
    ui.fireEvent.click(
      rendered.view.getByRole('button', { name: 'Skip for now' }),
    );
    assert.ok(rendered.view.getByText('How are you joining?'));
    await ui.act(async () =>
      discovery.resolve({ installed: false, candidates: [] }),
    );
    assert.ok(rendered.view.getByText('How are you joining?'));
  } finally {
    window.setTimeout = original;
  }
});

test('phrase preparation has an accessible back action outside the disabled controls', async () => {
  const h = await harness();
  const preparation = deferred<{ backupAlias: string; phrase: string }>();
  const rendered = h.render(h.checkpoint, {
    bridge: { ...h.bridge, prepareOwnerBackup: () => preparation.promise },
  });
  ui.fireEvent.click(
    rendered.view.getByRole('button', { name: 'Show recovery phrase' }),
  );
  const back = await rendered.view.findByRole('button', {
    name: 'Back to recovery options',
  });
  assert.equal(back.closest('[inert]'), null);
  ui.fireEvent.click(back);
  await ui.act(async () =>
    preparation.resolve({ backupAlias: 'paper', phrase: 'late secret' }),
  );
  assert.equal(rendered.view.queryByText('late secret'), null);
  assert.ok(
    rendered.view.getByRole('button', { name: 'Show recovery phrase' }),
  );
});

test('recovery phrase commit shows a saving label while waiting for confirmation', async () => {
  const h = await harness();
  const commit = deferred<Awaited<ReturnType<Bridge['commitOwnerBackup']>>>();
  const rendered = h.render(
    { ...h.checkpoint, managedLocal: false },
    {
      bridge: { ...h.bridge, commitOwnerBackup: () => commit.promise },
    },
  );
  ui.fireEvent.click(
    rendered.view.getByRole('button', { name: 'Show my phrase' }),
  );
  ui.fireEvent.click(
    await rendered.view.findByRole('button', {
      name: 'I have written this down',
    }),
  );
  ui.fireEvent.click(rendered.view.getByRole('button', { name: 'Done' }));
  const saving = await rendered.view.findByRole('button', {
    name: 'Saving...',
  });
  assert.equal((saving as HTMLButtonElement).disabled, true);
  await ui.act(async () => commit.resolve({ applied: true }));
  await rendered.view.findByRole('button', { name: 'Show my phrase' });
});

test('multiple groups refresh the catalog and preserve a selected unavailable vault across restart', async () => {
  const h = await harness();
  const groups = ['one', 'two'].map((alias, index) => ({
    alias,
    accountAlias: 'personal',
    teamIdHex: `03${String(index + 1).repeat(64)}`,
    kind: 'named' as const,
    name: alias,
    active: true,
  }));
  const forces: Array<boolean | undefined> = [];
  const checkpoint = {
    ...h.checkpoint,
    path: 'invited' as const,
    managedLocal: false,
    state: 'waiting' as const,
  };
  const rendered = h.render(checkpoint, {
    location: { kind: 'first-run', path: 'invited', step: 'waiting' },
    snapshot: h.complete,
    bridge: {
      ...h.bridge,
      discoverGroups: async () => ({ accountAlias: 'personal', groups }),
    },
    onRefreshSnapshot: async (force) => {
      forces.push(force);
      return h.complete;
    },
  });
  ui.fireEvent.click(rendered.view.getByRole('button', { name: 'Check now' }));
  await rendered.view.findByText('Choose an existing team');
  assert.deepEqual(forces, [true]);
  ui.fireEvent.click(rendered.view.getByRole('button', { name: 'Open “two”' }));
  assert.ok(rendered.view.getByRole('button', { name: 'Retry loading team' }));
  const saved = h.saved()!;
  assert.equal(saved.selectedGroup?.teamIdHex, groups[1].teamIdHex);
  assert.equal(saved.added, false);
  ui.cleanup();
  const resumed = h.render(saved, {
    snapshot: h.complete,
    location: { kind: 'first-run', path: 'invited', step: 'waiting' },
  });
  assert.ok(resumed.view.getByText('two'));
  assert.ok(resumed.view.getByRole('button', { name: 'Retry loading team' }));
});

test('zero groups still force catalog reconciliation', async () => {
  const h = await harness();
  let refreshes = 0;
  const rendered = h.render(
    { ...h.checkpoint, path: 'invited', managedLocal: false, state: 'waiting' },
    {
      location: { kind: 'first-run', path: 'invited', step: 'waiting' },
      bridge: {
        ...h.bridge,
        discoverGroups: async () => ({ accountAlias: 'personal', groups: [] }),
      },
      onRefreshSnapshot: async (force) => {
        assert.equal(force, true);
        refreshes++;
        return h.complete;
      },
    },
  );
  ui.fireEvent.click(rendered.view.getByRole('button', { name: 'Check now' }));
  await rendered.view.findByText(/No active teams found yet/);
  assert.equal(refreshes, 1);
});

test('local-server retry probes again despite a stale failed connectivity snapshot', async () => {
  const h = await harness();
  let probes = 0;
  let refreshes = 0;
  const failure = {
    code: 'io',
    message: 'Service stopped',
    ambiguous: false,
    fatal: false,
    retryable: true,
  };
  const stale = {
    ...h.complete,
    servers: h.complete.servers.map((server) => ({
      ...server,
      passiveStatus: {
        status: 'failed' as const,
        source: 'describe-server-status' as const,
        error: failure,
      },
    })),
  };
  const rendered = h.render(h.initialFirstRun('own', 'local'), {
    managedProfile: 'personal',
    snapshot: stale,
    location: { kind: 'first-run', step: 'local', path: 'own' },
    bridge: {
      ...h.bridge,
      describeServerStatus: async (_profile, fresh) => {
        assert.equal(
          fresh,
          true,
          'the native catalog cache must also be bypassed',
        );
        probes++;
        if (probes === 1) throw failure;
        return {
          profile: 'personal',
          configuredProbe: 'localhost:4430',
          host: h.checkpoint.profile!,
          leaseRequired: false,
          leaseExpiresAt: null,
          chatSupported: false,
          compatibility: { status: 'not-required' },
        };
      },
    },
    onRefreshSnapshot: async (force) => {
      assert.equal(force, true);
      refreshes++;
      return h.complete;
    },
  });
  await rendered.view.findByText('Service stopped');
  ui.fireEvent.click(
    rendered.view.getByRole('button', { name: 'Check again' }),
  );
  await rendered.view.findByText('Ready');
  assert.equal(probes, 2);
  assert.equal(refreshes, 1);
  assert.equal(
    (
      rendered.view.getByRole('button', {
        name: 'Continue',
      }) as HTMLButtonElement
    ).disabled,
    false,
  );
});

test('a native receipt confirms a lost reply without replaying account creation', async () => {
  const h = await harness();
  let mutations = 0;
  let attemptId: string | undefined;
  const rendered = h.render(
    { ...h.checkpoint, account: undefined, state: 'account' },
    {
      snapshot: h.complete,
      onRefreshSnapshot: async () => h.complete,
      bridge: {
        ...h.bridge,
        runFirstRunAccountOperation: async (request) => {
          mutations++;
          attemptId = request.attempt.id;
          assert.equal(h.saved()?.provisioning?.id, attemptId);
          throw {
            code: 'ambiguous',
            message: 'Lost desktop reply',
            ambiguous: true,
            retryable: false,
            fatal: false,
          };
        },
        firstRunOperationStatus: async (attempt) => {
          assert.equal(attempt.id, attemptId);
          assert.equal(attempt.hostId, h.checkpoint.profile!.hostId);
          return 'complete';
        },
      },
    },
  );
  ui.fireEvent.change(rendered.view.getByPlaceholderText('yourname'), {
    target: { value: 'personal' },
  });
  ui.fireEvent.click(
    rendered.view.getByRole('button', { name: 'Create my account' }),
  );
  await rendered.view.findByRole('button', { name: 'Show recovery phrase' });
  assert.equal(mutations, 1);
  assert.equal(h.saved()?.state, 'protect');
});

for (const cancellation of ['conceal', 'hidden', 'readiness'] as const) {
  test(`phrase preparation releases busy ownership on ${cancellation} and ignores its late result`, async () => {
    const h = await harness();
    const old = deferred<{ backupAlias: string; phrase: string }>();
    const next = deferred<{ backupAlias: string; phrase: string }>();
    let preparations = 0;
    const rendered = h.render(h.checkpoint, {
      bridge: {
        ...h.bridge,
        prepareOwnerBackup: () =>
          ++preparations === 1 ? old.promise : next.promise,
      },
      onRetryAgent: async () => {},
    });
    ui.fireEvent.click(
      rendered.view.getByRole('button', { name: 'Show recovery phrase' }),
    );
    await ui.waitFor(() =>
      assert.ok(
        rendered.view.container
          .querySelector('[data-setup-controls]')
          ?.hasAttribute('inert'),
      ),
    );
    if (cancellation === 'conceal') rendered.rerender({ concealSignal: 1 });
    else if (cancellation === 'readiness')
      rendered.rerender({ agentReady: false });
    else {
      Object.defineProperty(document, 'visibilityState', {
        configurable: true,
        value: 'hidden',
      });
      ui.fireEvent(document, new Event('visibilitychange'));
      Object.defineProperty(document, 'visibilityState', {
        configurable: true,
        value: 'visible',
      });
    }
    await ui.waitFor(() =>
      assert.equal(
        rendered.view.container
          .querySelector('[data-setup-controls]')
          ?.hasAttribute('inert'),
        false,
      ),
    );
    if (cancellation === 'readiness') {
      assert.ok(rendered.view.getByRole('button', { name: 'Retry setup' }));
      rendered.rerender({ agentReady: true });
    }
    assert.equal(h.saved()?.state, 'protect');
    ui.fireEvent.click(
      rendered.view.getByRole('button', { name: 'Show recovery phrase' }),
    );
    await ui.waitFor(() => assert.equal(preparations, 2));
    await ui.act(async () => {
      old.resolve({ backupAlias: 'paper', phrase: 'OLD SECRET' });
    });
    assert.equal(rendered.view.queryByText('OLD SECRET'), null);
    assert.ok(
      rendered.view.container
        .querySelector('[data-setup-controls]')
        ?.hasAttribute('inert'),
      'old completion must not release the new operation',
    );
    await ui.act(async () => {
      next.reject(new Error('Preparation failed'));
    });
    await ui.waitFor(() =>
      assert.equal(
        rendered.view.container
          .querySelector('[data-setup-controls]')
          ?.hasAttribute('inert'),
        false,
      ),
    );
    assert.ok(
      rendered.view.getByRole('button', { name: 'Show recovery phrase' }),
    );
  });
}

test('acknowledged signup is persisted before refresh and resumes read-only after restart', async () => {
  const h = await harness();
  const inventory = deferred<typeof h.complete>();
  let mutations = 0;
  let refreshes = 0;
  const bridge = {
    ...h.bridge,
    createFirstRunAccount: async () => {
      mutations++;
      return { applied: true };
    },
  };
  const rendered = h.render(
    { ...h.checkpoint, state: 'account', account: undefined },
    {
      bridge,
      onRefreshSnapshot: async () => {
        refreshes++;
        return inventory.promise;
      },
    },
  );
  ui.fireEvent.change(rendered.view.getByPlaceholderText('yourname'), {
    target: { value: 'personal' },
  });
  ui.fireEvent.change(rendered.view.getByPlaceholderText('Your device'), {
    target: { value: 'Mac' },
  });
  ui.fireEvent.click(
    rendered.view.getByRole('button', { name: 'Create my account' }),
  );
  await rendered.view.findByText('Your account is connected');
  assert.equal(h.saved()?.state, 'identity-pending');
  assert.equal(h.saved()?.provisionedAccount?.alias, 'personal');
  await ui.act(async () => {
    inventory.reject(new Error('Catalog unavailable'));
  });
  await rendered.view.findByText('Catalog unavailable');
  assert.equal(
    rendered.view.queryByRole('button', { name: 'Create my account' }),
    null,
  );
  const pending = h.saved()!;
  ui.cleanup();
  const resumed = h.render(pending, {
    bridge,
    location: { kind: 'first-run', path: 'own', step: 'account' },
    onRefreshSnapshot: async () => {
      refreshes++;
      return h.complete;
    },
  });
  assert.ok(resumed.view.getByText('Your account is connected'));
  await resumed.view.findByRole('button', { name: 'Show recovery phrase' });
  assert.equal(mutations, 1);
  assert.equal(refreshes, 2);
  assert.equal(h.saved()?.account?.username, 'satoshi');
  assert.equal(h.saved()?.provisionedAccount, undefined);
});

for (const method of ['recovery', 'sso'] as const) {
  test(`${method} acknowledgement survives a failed identity refresh without offering the mutation again`, async () => {
    const h = await harness();
    let mutations = 0;
    const bridge: Bridge = {
      ...h.bridge,
      recoverOwnerAccount: async () => {
        mutations++;
        return { applied: true };
      },
      sso: async (...args) => {
        mutations++;
        return {
          ...(await h.bridge.sso(...args)),
          state: 'complete',
          serviceAccess: true,
        };
      },
    };
    const rendered = h.render(
      {
        ...h.checkpoint,
        account: undefined,
        managedLocal: false,
        state: method === 'recovery' ? 'existing' : 'account',
      },
      {
        bridge,
        onRefreshSnapshot: async () => {
          throw new Error('Identity unavailable');
        },
      },
    );
    ui.fireEvent.change(rendered.view.getByPlaceholderText('Your device'), {
      target: { value: 'Mac' },
    });
    if (method === 'recovery') {
      ui.fireEvent.change(rendered.view.getByLabelText('Account alias'), {
        target: { value: 'personal' },
      });
      ui.fireEvent.change(rendered.view.getByLabelText('Backup phrase'), {
        target: { value: 'secret recovery phrase' },
      });
      ui.fireEvent.click(
        rendered.view.getByRole('button', { name: 'Recover' }),
      );
    } else {
      ui.fireEvent.change(rendered.view.getByPlaceholderText('yourname'), {
        target: { value: 'personal' },
      });
      ui.fireEvent.click(
        rendered.view.getByText('Sign up with your organization'),
      );
      ui.fireEvent.click(
        rendered.view.getByRole('button', {
          name: 'Continue with organization',
        }),
      );
    }
    await rendered.view.findByText('Identity unavailable');
    assert.equal(h.saved()?.state, 'identity-pending');
    assert.equal(h.saved()?.provisionedAccount?.alias, 'personal');
    assert.equal(
      window.localStorage
        .getItem(h.FIRST_RUN_CHECKPOINT_KEY)
        ?.includes('secret recovery phrase'),
      false,
    );
    ui.fireEvent.click(
      rendered.view.getByRole('button', { name: 'Retry loading account' }),
    );
    await rendered.view.findByText('Identity unavailable');
    assert.equal(mutations, 1);
  });
}

for (const method of ['copy', 'pair', 'resume-pair'] as const) {
  test(`CLI ${method} acknowledgement is retained when identity inventory stays unavailable`, async () => {
    const h = await harness();
    let mutations = 0;
    const refreshForces: Array<boolean | undefined> = [];
    const bridge: Bridge = {
      ...h.bridge,
      discoverGoProfiles: async () => ({
        installed: true,
        candidates: [
          {
            candidateId: 'cli',
            username: 'personal',
            hostId: h.checkpoint.profile!.hostId,
            userId: `01${'3'.repeat(64)}`,
            deviceId: `03${'4'.repeat(64)}`,
            role: 'owner',
            storageKind: 'macos-keychain',
            hidden: false,
            provisional: false,
            pairable: true,
            copyable: true,
          },
        ],
      }),
      checkAndAddGoProfile: async (_candidate, _host, profile) => ({
        ...h.checkpoint.profile!,
        profile,
      }),
      copyGoProfileDevice: async (...args) => {
        mutations++;
        return h.bridge.copyGoProfileDevice(...args);
      },
      acceptGoProfilePairing: async (...args) => {
        mutations++;
        return h.bridge.acceptGoProfilePairing(...args);
      },
      resumeGoProfilePairing: async (_candidate, _profile, alias) => {
        mutations++;
        return {
          alias,
          deviceId: `04${'6'.repeat(64)}`,
          userChainSequence: 15,
        };
      },
    };
    const rendered = h.render(h.initialFirstRun('own', 'who'), {
      bridge,
      onRefreshSnapshot: async (force) => {
        refreshForces.push(force);
        return h.unknown;
      },
    });
    await rendered.view.findByText('Set up FOKS');
    await rendered.view.findByRole('button', {
      name: /Selected account personal/,
    });
    ui.fireEvent.click(rendered.view.getByRole('button', { name: 'Continue' }));
    ui.fireEvent.click(
      rendered.view.getByRole('button', {
        name: 'Use the official FOKS server',
      }),
    );
    ui.fireEvent.click(rendered.view.getByRole('button', { name: 'Continue' }));
    await rendered.view.findByRole('button', { name: 'Details' });
    ui.fireEvent.click(rendered.view.getByRole('button', { name: 'Continue' }));
    // Three sign-in methods are available, so a method must be selected before
    // the footer action appears.
    ui.fireEvent.click(
      rendered.view.getByRole('radio', {
        name:
          method === 'copy'
            ? /Import this device’s FOKS CLI credentials/
            : /Use the CLI to approve this as a new device/,
      }),
    );
    if (method === 'copy')
      ui.fireEvent.click(
        rendered.view.getByRole('button', { name: 'Import credentials' }),
      );
    else {
      ui.fireEvent.change(rendered.view.getByLabelText('This device’s name'), {
        target: { value: 'Mac' },
      });
      if (method === 'pair')
        ui.fireEvent.change(rendered.view.getByLabelText('Pairing phrase'), {
          target: { value: 'secret pairing phrase' },
        });
      ui.fireEvent.click(
        rendered.view.getByRole('button', {
          name: method === 'pair' ? 'Accept pairing' : 'Resume pairing',
        }),
      );
    }
    await rendered.view.findByRole('button', { name: 'Retry loading account' });
    assert.equal(h.saved()?.provisionedAccount?.alias, 'personal');
    assert.equal(
      window.localStorage
        .getItem(h.FIRST_RUN_CHECKPOINT_KEY)
        ?.includes('secret pairing phrase'),
      false,
    );
    assert.equal(mutations, 1);
    assert.deepEqual(refreshForces, [true]);
  });
}

test('late identity refresh cannot complete onboarding after readiness is lost', async () => {
  const h = await harness();
  const result = deferred<typeof h.complete>();
  const pending = h.transitionFirstRun(
    { ...h.checkpoint, account: undefined, state: 'account' },
    {
      type: 'account-provisioned',
      alias: 'personal',
      deviceName: 'Mac',
    },
  );
  const rendered = h.render(pending, {
    onRefreshSnapshot: () => result.promise,
    onRetryAgent: async () => {},
  });
  rendered.rerender({ agentReady: false });
  await ui.act(async () => {
    result.resolve(h.complete);
  });
  assert.equal(h.saved()?.state, 'identity-pending');
  assert.ok(rendered.view.getByRole('button', { name: 'Retry setup' }));
  rendered.rerender({
    agentReady: true,
    onRefreshSnapshot: async () => h.complete,
  });
  ui.fireEvent.click(
    rendered.view.getByRole('button', { name: 'Retry loading account' }),
  );
  await rendered.view.findByRole('button', { name: 'Show recovery phrase' });
});

test('the development bridge supplies matching identity facts after pending setup is restored', async () => {
  const h = await harness();
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const { loadSnapshot } = (await vite.ssrLoadModule(
    '/src/bridge.ts',
  )) as typeof import('../src/bridge');
  const { resolveProvisionedIdentity } = (await vite.ssrLoadModule(
    '/src/first-run-identity.ts',
  )) as typeof import('../src/first-run-identity');
  const pending = h.transitionFirstRun(
    { ...h.checkpoint, state: 'account', account: undefined },
    {
      type: 'account-provisioned',
      alias: 'personal',
      deviceName: 'Mac',
    },
  );
  window.localStorage.setItem(
    h.FIRST_RUN_CHECKPOINT_KEY,
    h.encodeFirstRunCheckpoint(pending),
  );
  const refreshed = await loadSnapshot(mockBridge());
  assert.equal(resolveProvisionedIdentity(refreshed, pending).state, 'protect');
});

test('pending identity survives an empty refresh and is adopted from authoritative inventory on remount', async () => {
  const h = await harness();
  const pending = h.transitionFirstRun(
    { ...h.checkpoint, account: undefined, state: 'account' },
    {
      type: 'account-provisioned',
      alias: 'personal',
      deviceName: 'Mac',
    },
  );
  const rendered = h.render(pending);
  await rendered.view.findByText(/Couldn’t load your account details/);
  assert.ok(rendered.view.getByRole('button', { name: 'Finish later' }));
  assert.equal(h.saved()?.state, 'identity-pending');
  ui.cleanup();
  const resumed = h.render(pending, {
    snapshot: h.complete,
    location: { kind: 'first-run', path: 'own', step: 'who' },
  });
  assert.ok(resumed.view.getByRole('button', { name: 'Show recovery phrase' }));
  assert.equal(
    resumed.view.queryByRole('button', { name: 'Create my account' }),
    null,
  );
});

function pendingOperation(
  checkpoint: FirstRunCheckpoint,
  kind: 'signup' | 'copy',
  id: string,
): FirstRunCheckpoint {
  return {
    ...checkpoint,
    account: undefined,
    state: 'operation-pending',
    provisioning: {
      id,
      kind,
      alias: 'personal',
      deviceName: 'Mac',
      back: 'account',
    },
  };
}

test('allows starting over when unconfirmed setup has no resume path', async () => {
  const h = await harness();
  let mutations = 0;
  const absent = {
    ...h.complete,
    accounts: h.complete.accounts.filter((row) => row.server !== 'personal'),
  };
  const rendered = h.render(pendingOperation(h.checkpoint, 'copy', 'copy-1'), {
    snapshot: absent,
    onRefreshSnapshot: async () => absent,
    bridge: {
      ...h.bridge,
      listPendingOperations: async () => [],
      firstRunOperationStatus: async () => 'unknown' as const,
      runFirstRunAccountOperation: async () => {
        mutations++;
        return { applied: true };
      },
      createFirstRunAccount: async () => {
        mutations++;
        return { applied: true };
      },
    },
  });
  ui.fireEvent.click(
    await rendered.view.findByRole('button', { name: 'Start over' }),
  );
  assert.ok(rendered.view.getByRole('button', { name: 'Create my account' }));
  assert.equal(h.saved()?.state, 'account');
  assert.equal(h.saved()?.provisioning, undefined);
  assert.equal(mutations, 0);
});

test('shows a missing-server warning with the start-over action for an unconfirmed operation', async () => {
  const h = await harness();
  const missingProfile = {
    ...h.complete,
    servers: h.complete.servers.filter((row) => row.id !== 'personal'),
  };
  const rendered = h.render(
    pendingOperation(h.checkpoint, 'copy', 'copy-missing-server'),
    {
      snapshot: missingProfile,
      onRefreshSnapshot: async () => missingProfile,
      bridge: {
        ...h.bridge,
        firstRunOperationStatus: async () => 'unknown' as const,
      },
    },
  );
  const warning = await rendered.view.findByRole('alert');
  assert.ok(warning.classList.contains('warning'));
  assert.ok(
    ui
      .within(warning)
      .getByText('The server saved for this account is missing'),
  );
  assert.ok(
    ui
      .within(warning)
      .getByText(
        'The account you were creating could not be found on the server. This may happen because of a restart, server reset, or other error.',
      ),
  );
  assert.ok(ui.within(warning).getByRole('button', { name: 'Start over' }));
  assert.equal(
    rendered.view.getAllByRole('button', { name: 'Start over' }).length,
    1,
  );
});

test('an unreadable receipt still probes pending operations and reports the receipt failure', async () => {
  const h = await harness();
  const absent = {
    ...h.complete,
    accounts: h.complete.accounts.filter((row) => row.server !== 'personal'),
  };
  const rendered = h.render(
    pendingOperation(h.checkpoint, 'signup', 'signup-1'),
    {
      snapshot: absent,
      onRefreshSnapshot: async () => absent,
      bridge: {
        ...h.bridge,
        listPendingOperations: async () => [
          { kind: 'account-signup', alias: 'personal' },
        ],
        firstRunOperationStatus: async () => {
          throw {
            code: 'invalid-request',
            message:
              'This setup attempt does not match the current account or workspace.',
            ambiguous: false,
            retryable: false,
            fatal: false,
          };
        },
      },
    },
  );
  await rendered.view.findByRole('button', { name: 'Resume account setup' });
  assert.ok(
    rendered.view.getByText(
      /\(Details: This setup attempt does not match the current account or workspace\.\)/,
    ),
  );
  assert.ok(rendered.view.getByRole('button', { name: 'Start over' }));
  assert.equal(
    rendered.view.queryByRole('button', {
      name: 'Continue with existing account',
    }),
    null,
  );
});

test('allows continuing with an existing account when a receipt cannot be read', async () => {
  const h = await harness();
  let mutations = 0;
  const rendered = h.render(
    pendingOperation(h.checkpoint, 'signup', 'signup-2'),
    {
      snapshot: h.complete,
      onRefreshSnapshot: async () => h.complete,
      bridge: {
        ...h.bridge,
        listPendingOperations: async () => [],
        firstRunOperationStatus: async () => {
          throw {
            code: 'io',
            message: 'The setup receipt could not be read.',
            ambiguous: false,
            retryable: true,
            fatal: false,
          };
        },
        runFirstRunAccountOperation: async () => {
          mutations++;
          return { applied: true };
        },
        createFirstRunAccount: async () => {
          mutations++;
          return { applied: true };
        },
        resumeFirstRunAccount: async () => {
          mutations++;
          return { applied: true };
        },
      },
    },
  );
  const adopt = await rendered.view.findByRole('button', {
    name: 'Continue with existing account',
  });
  assert.equal(h.saved()?.state, 'operation-pending');
  assert.equal(h.saved()?.account, undefined);
  assert.ok(
    rendered.view.getByText(
      /An account with this username already exists on this server/,
    ),
  );
  ui.fireEvent.click(adopt);
  await rendered.view.findByRole('button', { name: 'Show recovery phrase' });
  assert.equal(h.saved()?.state, 'protect');
  assert.equal(h.saved()?.account?.username, 'satoshi');
  assert.equal(mutations, 0);
});

test('shows option to set up a different account when identity is missing', async () => {
  const h = await harness();
  let mutations = 0;
  const absent = {
    ...h.complete,
    accounts: h.complete.accounts.filter((row) => row.server !== 'personal'),
  };
  const pending = h.transitionFirstRun(
    { ...h.checkpoint, account: undefined, state: 'account' },
    { type: 'account-provisioned', alias: 'personal', deviceName: 'Mac' },
  );
  const bridge: Bridge = {
    ...h.bridge,
    createFirstRunAccount: async () => {
      mutations++;
      return { applied: true };
    },
    runFirstRunAccountOperation: async () => {
      mutations++;
      return { applied: true };
    },
  };
  const rendered = h.render(pending, {
    bridge,
    snapshot: absent,
    onRefreshSnapshot: async () => absent,
  });
  ui.fireEvent.click(
    await rendered.view.findByRole('button', {
      name: 'Set up a different account',
    }),
  );
  assert.ok(rendered.view.getByRole('button', { name: 'Create my account' }));
  assert.equal(h.saved()?.state, 'account');
  assert.equal(h.saved()?.provisionedAccount, undefined);
  assert.equal(mutations, 0);
  ui.cleanup();
  const unavailable = h.render(pending, { bridge });
  await unavailable.view.findByText(/Couldn’t load your account details/);
  assert.ok(
    unavailable.view.getByRole('button', {
      name: 'Set up a different account',
    }),
  );
});

test('shows a missing-server warning with the different-account action for acknowledged setup', async () => {
  const h = await harness();
  const missingProfile = {
    ...h.complete,
    servers: h.complete.servers.filter((row) => row.id !== 'personal'),
  };
  const pending = h.transitionFirstRun(
    { ...h.checkpoint, account: undefined, state: 'account' },
    { type: 'account-provisioned', alias: 'personal', deviceName: 'Mac' },
  );
  const rendered = h.render(pending, {
    snapshot: missingProfile,
    onRefreshSnapshot: async () => missingProfile,
  });
  const warning = await rendered.view.findByRole('alert');
  assert.ok(warning.classList.contains('warning'));
  assert.ok(
    ui
      .within(warning)
      .getByText('The server saved for this account is missing'),
  );
  assert.ok(
    ui
      .within(warning)
      .getByText(
        'The account you were creating could not be found on the server. This may happen because of a restart, server reset, or other error.',
      ),
  );
  assert.ok(
    ui
      .within(warning)
      .getByRole('button', { name: 'Set up a different account' }),
  );
  assert.equal(
    rendered.view.getAllByRole('button', {
      name: 'Set up a different account',
    }).length,
    1,
  );
});

test('a pending operation is confirmed on mount without any click', async () => {
  const h = await harness();
  const rendered = h.render(
    pendingOperation(h.checkpoint, 'signup', 'signup-3'),
    {
      snapshot: h.complete,
      onRefreshSnapshot: async () => h.complete,
      bridge: {
        ...h.bridge,
        firstRunOperationStatus: async () => 'complete' as const,
      },
    },
  );
  await rendered.view.findByRole('button', { name: 'Show recovery phrase' });
  assert.equal(h.saved()?.state, 'protect');
  assert.equal(h.saved()?.provisioning, undefined);
});

test('cannot discard account setup while it is still running', async () => {
  const h = await harness();
  const rendered = h.render(
    pendingOperation(h.checkpoint, 'signup', 'signup-4'),
    {
      snapshot: h.complete,
      onRefreshSnapshot: async () => h.complete,
      bridge: {
        ...h.bridge,
        firstRunOperationStatus: async () => 'running' as const,
      },
    },
  );
  await rendered.view.findByText(/Account setup is still running/);
  assert.equal(
    rendered.view.queryByRole('button', { name: 'Start over' }),
    null,
  );
  assert.equal(
    (
      rendered.view.getByRole('button', {
        name: 'Start setup over',
      }) as HTMLButtonElement
    ).disabled,
    true,
  );
  assert.equal(h.saved()?.state, 'operation-pending');
});
