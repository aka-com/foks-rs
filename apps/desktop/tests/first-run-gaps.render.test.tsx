import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import type { Bridge } from '../src/bridge';
import type { FirstRunExperienceProps } from '../src/screens/first-run-screen';
import type { FirstRunCheckpoint } from '../src/first-run-state';
import type { Location } from '../src/location';
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

function never<T>(): Promise<T> {
  return new Promise<T>(() => {});
}

async function harness() {
  const screen = (await vite.ssrLoadModule(
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
  const model = (await vite.ssrLoadModule(
    '/src/model/index.ts',
  )) as typeof import('../src/model');
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
      children: createElement(screen.FirstRunExperience, props),
    });
  const mount = (
    saved: FirstRunCheckpoint | null,
    overrides: Partial<FirstRunExperienceProps>,
  ) => {
    if (saved)
      window.localStorage.setItem(
        state.FIRST_RUN_CHECKPOINT_KEY,
        state.encodeFirstRunCheckpoint(saved),
      );
    else window.localStorage.removeItem(state.FIRST_RUN_CHECKPOINT_KEY);
    let props: FirstRunExperienceProps = {
      bridge,
      world: unknown,
      location: {
        kind: 'first-run',
        path: 'own',
        step: saved?.state ?? 'who',
      },
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
  };
  return {
    ...state,
    accountAliasFor: screen.accountAliasFor,
    applyLease: model.applyLease,
    checkpoint,
    bridge,
    complete,
    unknown,
    render(
      saved = checkpoint,
      overrides: Partial<FirstRunExperienceProps> = {},
    ) {
      return mount(saved, overrides);
    },
    renderFresh(overrides: Partial<FirstRunExperienceProps> = {}) {
      return mount(null, overrides);
    },
  };
}

test('local setup without a managed server reports why it cannot continue', async () => {
  const h = await harness();
  const rendered = h.render(h.initialFirstRun('own', 'local'), {
    world: h.complete,
    onRefreshWorld: async () => h.complete,
  });
  await rendered.view.findByText(/No local server is running/);
  assert.ok(
    rendered.view.getByText(
      'No local server is running on this Mac. Connect to an existing server to continue.',
    ),
  );
  assert.equal(
    (
      rendered.view.getByRole('button', {
        name: 'Continue',
      }) as HTMLButtonElement
    ).disabled,
    true,
  );
  assert.equal(
    (
      rendered.view.getByRole('button', {
        name: 'Recover an existing account…',
      }) as HTMLButtonElement
    ).disabled,
    true,
  );
  assert.ok(
    rendered.view.getByRole('button', { name: 'Connect to another server…' }),
  );
});

test('account alias derivation trims, rejects unusable names, and bounds length', async () => {
  const h = await harness();
  assert.equal(h.accountAliasFor('日本語'), '');
  assert.equal(h.accountAliasFor('Ada Lovelace'), 'ada-lovelace');
  assert.equal(h.accountAliasFor('--x--'), 'x');
  assert.equal(h.accountAliasFor('a'.repeat(100)).length, 64);
});

test('a username with no usable characters blocks account creation', async () => {
  const h = await harness();
  let creates = 0;
  const rendered = h.render(
    {
      ...h.checkpoint,
      managedLocal: false,
      account: undefined,
      state: 'account',
    },
    {
      world: h.complete,
      onRefreshWorld: async () => h.complete,
      bridge: {
        ...h.bridge,
        createFirstRunAccount: async () => {
          creates++;
          return { applied: true };
        },
      },
    },
  );
  ui.fireEvent.change(rendered.view.getByPlaceholderText('yourname'), {
    target: { value: '日本語' },
  });
  assert.ok(
    rendered.view.getByText(
      'Username must contain at least one letter or number.',
    ),
  );
  const create = rendered.view.getByRole('button', {
    name: 'Create my account',
  }) as HTMLButtonElement;
  assert.equal(create.disabled, true);
  ui.fireEvent.click(create);
  await ui.act(async () => {});
  assert.equal(creates, 0);
});

test('an unknown step in the URL starts setup at the first question', async () => {
  const h = await harness();
  const location: Extract<Location, { kind: 'first-run' }> = {
    kind: 'first-run',
    path: 'own',
    step: 'bogus',
  };
  const rendered = h.renderFresh({
    location,
    world: h.complete,
    onRefreshWorld: async () => h.complete,
    bridge: { ...h.bridge, native: false },
  });
  assert.ok(rendered.view.getByText('How are you joining?'));
});

test('an unconfirmed account operation keeps the sidebar on the account step', async () => {
  const h = await harness();
  const pending: FirstRunCheckpoint = {
    ...h.checkpoint,
    account: undefined,
    state: 'operation-pending',
    provisioning: {
      id: 'signup-1',
      kind: 'signup',
      alias: 'personal',
      deviceName: 'Mac',
      back: 'account',
    },
  };
  const absent = {
    ...h.complete,
    accounts: h.complete.accounts.filter((row) => row.server !== 'personal'),
  };
  const rendered = h.render(pending, {
    world: absent,
    onRefreshWorld: async () => absent,
    bridge: {
      ...h.bridge,
      listPendingOperations: async () => [],
      firstRunOperationStatus: async () => 'unknown' as const,
    },
  });
  await ui.act(async () => {});
  const current = rendered.view.container.querySelector(
    '[aria-current="step"]',
  );
  assert.ok(current);
  assert.equal(current?.querySelector('.t')?.textContent, 'Create account');
});

test('shows why the Personal vault is unavailable and provides a link to server settings', async () => {
  const h = await harness();
  const lapsed = h.applyLease(h.complete, 'lapsed', 'personal');
  const seen: Location[] = [];
  const rendered = h.render(
    { ...h.checkpoint, state: 'local-done', backupCommitted: true },
    {
      world: lapsed,
      onRefreshWorld: async () => lapsed,
      onNavigate: (location) => seen.push(location),
    },
  );
  assert.ok(
    rendered.view.getByText(
      /Your Personal vault is unavailable \(check-in expired\)\./,
    ),
  );
  assert.equal(
    rendered.view.queryByRole('button', { name: 'Open Personal' }),
    null,
  );
  ui.fireEvent.click(
    rendered.view.getByRole('button', { name: 'Review server settings' }),
  );
  assert.deepEqual(seen.at(-1), {
    kind: 'settings',
    section: 'servers',
    profile: 'personal',
  });
  assert.ok(
    rendered.view.getByRole('button', {
      name: 'Retry loading Personal vault',
    }),
  );
});

test('checklist allows retrying Personal vault loading while the account is not yet loaded', async () => {
  const h = await harness();
  let refreshes = 0;
  const rendered = h.render(
    { ...h.checkpoint, state: 'checklist-own', backupCommitted: true },
    {
      world: h.unknown,
      onRefreshWorld: async () => {
        refreshes++;
        throw new Error('Inventory unavailable');
      },
    },
  );
  assert.equal(
    (
      rendered.view.getByRole('button', {
        name: 'Open Personal',
      }) as HTMLButtonElement
    ).disabled,
    true,
  );
  ui.fireEvent.click(
    rendered.view.getByRole('button', {
      name: 'Retry loading Personal vault',
    }),
  );
  await rendered.view.findByText('Inventory unavailable');
  assert.equal(refreshes, 1);
});

test('allows leaving setup during server verification, but disables leaving during passphrase setup', async () => {
  const h = await harness();
  const seen: Location[] = [];
  const address = h.render(
    {
      ...h.initialFirstRun('own', 'address'),
      serverAddress: 'localhost:4430',
    },
    {
      world: h.complete,
      onRefreshWorld: async () => h.complete,
      onNavigate: (location) => seen.push(location),
      bridge: {
        ...h.bridge,
        checkAndAddProfile: () =>
          never<Awaited<ReturnType<Bridge['checkAndAddProfile']>>>(),
      },
    },
  );
  ui.fireEvent.click(
    address.view.getByRole('button', { name: 'Use this server' }),
  );
  const leave = address.view.getByRole('button', {
    name: 'Leave setup',
  }) as HTMLButtonElement;
  assert.equal(leave.disabled, false);
  ui.fireEvent.click(leave);
  assert.deepEqual(seen.at(-1), { kind: 'all' });
  ui.cleanup();

  const protect = h.render(
    { ...h.checkpoint, managedLocal: false, state: 'protect' },
    {
      world: h.complete,
      onRefreshWorld: async () => h.complete,
      bridge: {
        ...h.bridge,
        setFirstRunPassphrase: () =>
          never<Awaited<ReturnType<Bridge['setFirstRunPassphrase']>>>(),
      },
    },
  );
  ui.fireEvent.change(protect.view.getByLabelText('Passphrase'), {
    target: { value: 'correct horse battery' },
  });
  ui.fireEvent.change(protect.view.getByLabelText('Confirm passphrase'), {
    target: { value: 'correct horse battery' },
  });
  ui.fireEvent.click(protect.view.getByRole('button', { name: 'Continue' }));
  assert.equal(
    (
      protect.view.getByRole('button', {
        name: 'Finish later',
      }) as HTMLButtonElement
    ).disabled,
    true,
  );
});
