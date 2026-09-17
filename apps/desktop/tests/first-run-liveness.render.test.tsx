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
    account: { alias: 'personal', username: 'rae', deviceName: 'Mac' },
  };
  const bridge: Bridge = {
    ...mockBridge(complete),
    native: true,
    firstRunFixture: undefined,
  };
  const controller = new ToastController();
  const element = (props: FirstRunExperienceProps) =>
    createElement(ToastProvider, {
      controller,
      children: createElement(FirstRunExperience, props),
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
        world: unknown,
        location: { kind: 'first-run', path: 'own', step: saved.state },
        onNavigate: () => {},
        onRefreshWorld: async () => unknown,
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

test('completed local setup explains unavailable inventory and offers a retry through failure and success', async () => {
  const h = await harness();
  let refreshes = 0;
  const saved = {
    ...h.checkpoint,
    state: 'local-done' as const,
    backupCommitted: true,
  };
  const rendered = h.render(saved, {
    onRefreshWorld: async () => {
      refreshes++;
      if (refreshes === 1) throw new Error('Inventory unavailable');
      rendered.rerender({ world: h.complete });
      return h.complete;
    },
  });
  assert.ok(rendered.view.getByText('Setup is complete'));
  assert.equal(rendered.view.queryByText('Your Personal vault is ready'), null);
  ui.fireEvent.click(
    rendered.view.getByRole('button', { name: 'Retry loading Personal' }),
  );
  await rendered.view.findByText('Inventory unavailable');
  assert.equal(h.saved()?.state, 'local-done');
  ui.fireEvent.click(
    rendered.view.getByRole('button', { name: 'Retry loading Personal' }),
  );
  await rendered.view.findByRole('button', { name: 'Open Personal' });
  assert.equal(refreshes, 2);
  assert.equal(h.saved()?.account?.alias, 'personal');
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
        rendered.view.container.querySelector('main')?.hasAttribute('inert'),
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
        rendered.view.container.querySelector('main')?.hasAttribute('inert'),
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
      rendered.view.container.querySelector('main')?.hasAttribute('inert'),
      'old completion must not release the new operation',
    );
    await ui.act(async () => {
      next.reject(new Error('Preparation failed'));
    });
    await ui.waitFor(() =>
      assert.equal(
        rendered.view.container.querySelector('main')?.hasAttribute('inert'),
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
      onRefreshWorld: async () => {
        refreshes++;
        return inventory.promise;
      },
    },
  );
  ui.fireEvent.change(rendered.view.getByPlaceholderText('yourname'), {
    target: { value: 'personal' },
  });
  ui.fireEvent.change(rendered.view.getByPlaceholderText('Your Mac'), {
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
    onRefreshWorld: async () => {
      refreshes++;
      return h.complete;
    },
  });
  assert.ok(resumed.view.getByText('Your account is connected'));
  ui.fireEvent.click(
    resumed.view.getByRole('button', { name: 'Retry loading account' }),
  );
  await resumed.view.findByRole('button', { name: 'Show recovery phrase' });
  assert.equal(mutations, 1);
  assert.equal(refreshes, 2);
  assert.equal(h.saved()?.account?.username, 'rae');
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
        onRefreshWorld: async () => {
          throw new Error('Identity unavailable');
        },
      },
    );
    ui.fireEvent.change(rendered.view.getByPlaceholderText('Your Mac'), {
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
    const rendered = h.render(h.initialFirstRun('own', 'who'), { bridge });
    await rendered.view.findByText('Select an FOKS account');
    ui.fireEvent.click(rendered.view.getByRole('radio', { name: /personal/ }));
    ui.fireEvent.click(rendered.view.getByRole('button', { name: 'Continue' }));
    ui.fireEvent.click(
      rendered.view.getByRole('button', {
        name: 'Use the official FOKS server',
      }),
    );
    ui.fireEvent.click(
      rendered.view.getByRole('button', { name: 'Use this server' }),
    );
    await rendered.view.findByRole('button', { name: 'Details' });
    ui.fireEvent.click(rendered.view.getByRole('button', { name: 'Continue' }));
    if (method === 'copy')
      ui.fireEvent.click(
        rendered.view.getByRole('button', { name: 'Copy existing device' }),
      );
    else {
      ui.fireEvent.change(rendered.view.getByLabelText('Pairing device name'), {
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
    onRefreshWorld: () => result.promise,
    onRetryAgent: async () => {},
  });
  ui.fireEvent.click(
    rendered.view.getByRole('button', { name: 'Retry loading account' }),
  );
  rendered.rerender({ agentReady: false });
  await ui.act(async () => {
    result.resolve(h.complete);
  });
  assert.equal(h.saved()?.state, 'identity-pending');
  assert.ok(rendered.view.getByRole('button', { name: 'Retry setup' }));
  rendered.rerender({
    agentReady: true,
    onRefreshWorld: async () => h.complete,
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
  const { loadWorld } = (await vite.ssrLoadModule(
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
  const refreshed = await loadWorld(mockBridge());
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
  ui.fireEvent.click(
    rendered.view.getByRole('button', { name: 'Retry loading account' }),
  );
  await rendered.view.findByText(/The account details are not available yet/);
  assert.equal(h.saved()?.state, 'identity-pending');
  ui.cleanup();
  const resumed = h.render(pending, {
    world: h.complete,
    location: { kind: 'first-run', path: 'own', step: 'who' },
  });
  assert.ok(resumed.view.getByRole('button', { name: 'Show recovery phrase' }));
  assert.equal(
    resumed.view.queryByRole('button', { name: 'Create my account' }),
    null,
  );
});
