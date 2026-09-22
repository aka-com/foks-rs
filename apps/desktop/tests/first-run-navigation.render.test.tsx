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

/** Restart setup asks before it discards; the dialog carries Start over. */
function restartSetup(view: ReturnType<typeof ui.render>): void {
  ui.fireEvent.click(view.getByRole('button', { name: 'Restart setup' }));
  ui.fireEvent.click(
    ui
      .within(view.getByRole('alertdialog', { name: 'Restart setup?' }))
      .getByRole('button', { name: 'Start over' }),
  );
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
  const { OverlayProvider } = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives');
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
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
      server.profileName === 'personal'
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
  // Setup confirms Restart setup in a sheet, as the shell mounts it.
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

test('account name defaults to the macOS short account name', async () => {
  const h = await harness();
  const r = h.render(
    {
      ...h.checkpoint,
      account: undefined,
      state: 'existing',
    },
    {
      bridge: {
        ...h.bridge,
        appInfo: async () => ({
          version: '0.4.0',
          agentSocket: '/private/foks/agent.sock',
          userName: 'example',
        }),
      },
    },
  );
  await ui.waitFor(() =>
    assert.equal(
      (r.view.getByLabelText('Your name') as HTMLInputElement).value,
      'example',
    ),
  );
});

test('resumed sign-in and create share a deterministic server Back destination', async () => {
  const h = await harness();
  const r = h.render({
    ...h.checkpoint,
    managedLocal: false,
    account: undefined,
    state: 'existing',
  });
  ui.fireEvent.click(
    r.view.getByRole('radio', { name: /Create a new account/ }),
  );
  ui.fireEvent.click(r.view.getByRole('button', { name: 'Back' }));
  assert.equal(h.saved()?.state, 'checked');
});
test('the account pages are numbered sections and a lone sign-in method is preselected', async () => {
  const h = await harness();
  const steps = (view: ReturnType<typeof ui.render>) =>
    [...view.container.querySelectorAll('.sec.step')].map((el) => [
      el.querySelector('.n')?.textContent,
      el.textContent?.slice(1),
    ]);
  // Sign-in off the managed path: setup method, sign-in method, then the
  // account rows. Without a CLI candidate recovery is the only way in, so it
  // is chosen and its phrase row is drawn with the account rows.
  const r = h.render({
    ...h.checkpoint,
    managedLocal: false,
    account: undefined,
    state: 'existing',
  });
  assert.deepEqual(steps(r.view), [
    ['1', 'Setup method'],
    ['2', 'Sign in method'],
    ['3', 'Account and device'],
  ]);
  const methods = r.view.getByRole('radiogroup', { name: 'Sign-in method' });
  assert.deepEqual(
    ui
      .within(methods)
      .getAllByRole('radio')
      .map((el) => el.getAttribute('aria-checked')),
    ['true'],
  );
  const form = r.view
    .getByLabelText('Recovery phrase')
    .closest<HTMLElement>('.inset');
  assert.ok(form);
  assert.ok(ui.within(form).getByLabelText('Your name'));
  assert.ok(ui.within(form).getByLabelText('This device’s name'));
  assert.ok(r.view.getByRole('button', { name: 'Recover' }));
  assert.equal(r.view.queryByRole('button', { name: 'Continue' }), null);
  // Create: no sign-in method group, and the account rows come second.
  ui.fireEvent.click(
    r.view.getByRole('radio', { name: /Create a new account/ }),
  );
  assert.deepEqual(steps(r.view), [
    ['1', 'Setup method'],
    ['2', 'Account and device'],
  ]);
  assert.equal(
    r.view.queryByRole('radiogroup', { name: 'Sign-in method' }),
    null,
  );
  assert.equal(r.view.queryByLabelText('Recovery phrase'), null);
  assert.ok(r.view.getByRole('button', { name: 'Create my account' }));
  ui.cleanup();
  // The managed-local page has no setup-method group, so the sign-in method
  // is its first section.
  const local = h.render({
    ...h.checkpoint,
    account: undefined,
    state: 'existing',
  });
  assert.ok(
    local.view.getByRole('heading', {
      name: 'Add this device to your account',
    }),
  );
  assert.deepEqual(steps(local.view), [
    ['1', 'Sign in method'],
    ['2', 'Account and device'],
  ]);
  assert.ok(local.view.getByLabelText('Recovery phrase'));
  assert.ok(local.view.getByRole('button', { name: 'Recover' }));
});
test('local recover/create choices cannot create a Back cycle', async () => {
  const h = await harness();
  const r = h.render({ ...h.checkpoint, account: undefined, state: 'account' });
  ui.fireEvent.click(
    r.view.getByRole('button', { name: /Recover an existing account/ }),
  );
  ui.fireEvent.click(
    r.view.getByRole('button', { name: 'Create a new account' }),
  );
  ui.fireEvent.click(r.view.getByRole('button', { name: 'Back' }));
  assert.equal(h.saved()?.state, 'local');
});
test('entry without prerequisites and changed navigation resolve to valid steps', async () => {
  const h = await harness();
  const r = h.render(h.initialFirstRun('own', 'account'), {
    bridge: { ...h.bridge, native: false },
  });
  assert.ok(r.view.getByRole('heading', { name: 'How are you joining?' }));
  ui.cleanup();
  const next = h.render({
    ...h.checkpoint,
    managedLocal: false,
    account: undefined,
    state: 'account',
  });
  next.rerender({
    location: { kind: 'first-run', path: 'own', step: 'checked' },
  });
  assert.ok(next.view.getByRole('heading', { name: 'Select a server' }));
});

test('restarting setup is confirmed first and then drops retained attempts', async () => {
  const h = await harness();
  const { retainSetup, retainedSetups } = (await vite.ssrLoadModule(
    '/src/first-run-recovery.ts',
  )) as typeof import('../src/first-run-recovery');
  const pending = h.transitionFirstRun(
    { ...h.checkpoint, account: undefined, state: 'account' },
    { type: 'account-provisioned', alias: 'personal', deviceName: 'Device' },
  );
  retainSetup(pending);
  const r = h.render(pending, { bridge: { ...h.bridge, native: false } });
  await r.view.findByText(/Couldn’t load your account details/);
  ui.fireEvent.click(r.view.getByRole('button', { name: 'Restart setup' }));
  assert.ok(
    ui
      .within(r.view.getByRole('alertdialog'))
      .getByText(
        'The setup in progress on this device will be discarded. If an account was already created on the server, you will need a recovery key to reconnect to it.',
      ),
  );
  ui.fireEvent.click(r.view.getByRole('button', { name: 'Cancel' }));
  assert.equal(h.saved()?.state, 'identity-pending');
  assert.equal(retainedSetups().length, 1);
  restartSetup(r.view);
  assert.equal(h.saved()?.state, 'who');
  assert.ok(r.view.getByRole('heading', { name: 'How are you joining?' }));
  assert.deepEqual(retainedSetups(), []);
});

test('resumable recovery without its phrase can start a different setup', async () => {
  const h = await harness();
  const absent = {
    ...h.complete,
    accounts: h.complete.accounts.filter((row) => row.server !== 'personal'),
  };
  const r = h.render(
    {
      ...h.checkpoint,
      account: undefined,
      state: 'operation-pending',
      provisioning: {
        id: 'recover-1',
        kind: 'recovery',
        alias: 'personal',
        deviceName: 'Device',
        back: 'existing',
      },
    },
    {
      snapshot: absent,
      onRefreshSnapshot: async () => absent,
      bridge: {
        ...h.bridge,
        firstRunOperationStatus: async () => 'unknown',
        listPendingOperations: async () => [
          { kind: 'account-recovery', alias: 'personal', target: undefined },
        ],
      },
    },
  );
  const resume = await r.view.findByRole('button', {
    name: 'Resume account setup',
  });
  assert.equal((resume as HTMLButtonElement).disabled, true);
  restartSetup(r.view);
  assert.equal(h.saved()?.state, 'who');
  assert.equal(h.saved()?.provisioning, undefined);
  assert.equal(
    r.view.queryByRole('button', { name: 'Resume account setup' }),
    null,
  );
});

test('canceling organization signup releases ordinary account creation', async () => {
  const h = await harness();
  let cancelled = false;
  const operationId = 'a'.repeat(32);
  const r = h.render(
    {
      ...h.checkpoint,
      managedLocal: false,
      account: undefined,
      state: 'account',
      sso: { operationId, alias: 'personal', hardware: false },
    },
    {
      bridge: {
        ...h.bridge,
        sso: async (_profile, alias, action) => {
          if (action.action === 'cancel') cancelled = true;
          return {
            operationId,
            accountAlias: alias,
            purpose: 'signup',
            accountStatus: null,
            state: cancelled ? 'cancelled' : 'ready',
            browserAvailable: false,
            expiresAtMs: 0,
            serviceAccess: false,
          };
        },
      },
    },
  );
  ui.fireEvent.click(
    await r.view.findByRole('button', { name: 'Cancel sign-in' }),
  );
  await ui.waitFor(() => assert.equal(h.saved()?.sso, undefined));
  assert.equal(
    (
      r.view.getByRole('radio', {
        name: /Create a new account/,
      }) as HTMLButtonElement
    ).disabled,
    false,
  );
});

test('changing servers drops an automatically selected CLI host binding', async () => {
  const h = await harness();
  let goChecks = 0;
  let ordinaryChecks = 0;
  const candidate = {
    candidateId: 'candidate',
    username: 'cli-owner',
    hostId: h.checkpoint.profile!.hostId,
    userId: '01' + 'cd'.repeat(32),
    deviceId: '03' + 'ef'.repeat(32),
    role: 'owner',
    storageKind: 'macos-keychain' as const,
    hidden: false,
    provisional: false,
    pairable: true,
    copyable: true,
  };
  const r = h.render(
    {
      ...h.checkpoint,
      managedLocal: false,
      account: undefined,
      state: 'account',
    },
    {
      bridge: {
        ...h.bridge,
        discoverGoProfiles: async () => ({
          installed: true,
          candidates: [candidate],
        }),
        checkAndAddGoProfile: async () => {
          goChecks++;
          return h.checkpoint.profile!;
        },
        checkAndAddProfile: async () => {
          ordinaryChecks++;
          return h.checkpoint.profile!;
        },
      },
    },
  );
  await ui.act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 30));
  });
  ui.fireEvent.click(r.view.getByRole('button', { name: 'Back' }));
  ui.fireEvent.change(r.view.getByLabelText('Server address'), {
    target: { value: 'other.example:4430' },
  });
  ui.fireEvent.click(r.view.getByRole('button', { name: 'Continue' }));
  await ui.waitFor(() => assert.equal(ordinaryChecks, 1));
  assert.equal(goChecks, 0);
});

test('restart during identity loading ignores the old read when it completes', async () => {
  const h = await harness();
  const read = deferred<typeof h.complete>();
  const pending = h.transitionFirstRun(
    { ...h.checkpoint, account: undefined, state: 'account' },
    { type: 'account-provisioned', alias: 'personal', deviceName: 'Device' },
  );
  const r = h.render(pending, {
    bridge: { ...h.bridge, native: false },
    onRefreshSnapshot: () => read.promise,
  });
  await r.view.findByRole('button', { name: 'Loading account details…' });
  restartSetup(r.view);
  await ui.act(async () => {
    read.resolve(h.complete);
  });
  assert.equal(h.saved()?.state, 'who');
  assert.ok(r.view.getByRole('heading', { name: 'How are you joining?' }));
});

test('a retained attempt on the same target is reconciled instead of creating again', async () => {
  const h = await harness();
  const { retainSetup } = (await vite.ssrLoadModule(
    '/src/first-run-recovery.ts',
  )) as typeof import('../src/first-run-recovery');
  retainSetup({
    ...h.checkpoint,
    account: undefined,
    state: 'operation-pending',
    provisioning: {
      id: 'earlier-attempt',
      alias: 'personal',
      deviceName: 'Device',
      kind: 'signup',
      back: 'account',
    },
  });
  let writes = 0;
  const r = h.render(
    {
      ...h.checkpoint,
      managedLocal: false,
      account: undefined,
      state: 'account',
    },
    {
      bridge: {
        ...h.bridge,
        firstRunOperationStatus: async () => 'unknown',
        runFirstRunAccountOperation: async () => {
          writes++;
          return { applied: true };
        },
        createFirstRunAccount: async () => {
          writes++;
          return { applied: true };
        },
      },
    },
  );
  ui.fireEvent.change(r.view.getByPlaceholderText('yourname'), {
    target: { value: 'personal' },
  });
  ui.fireEvent.click(r.view.getByRole('button', { name: 'Create my account' }));
  await ui.waitFor(() =>
    assert.equal(h.saved()?.provisioning?.id, 'earlier-attempt'),
  );
  assert.equal(writes, 0);
});

test('failed recovery-reference persistence prevents reset from dropping the checkpoint', async () => {
  const h = await harness();
  const pending = h.transitionFirstRun(
    { ...h.checkpoint, account: undefined, state: 'account' },
    { type: 'account-provisioned', alias: 'personal', deviceName: 'Device' },
  );
  const r = h.render(pending);
  await r.view.findByText(/Couldn’t load your account details/);
  const prototype = Object.getPrototypeOf(window.localStorage) as Storage;
  const descriptor = Object.getOwnPropertyDescriptor(prototype, 'setItem')!;
  const original = window.localStorage.setItem.bind(window.localStorage);
  prototype.setItem = function (key: string, value: string) {
    if (key === 'foks.setup-recovery.v1') throw new Error('Storage is full');
    original(key, value);
  };
  try {
    restartSetup(r.view);
    assert.ok(r.view.getByText('Storage is full'));
    assert.equal(h.saved()?.state, 'identity-pending');
  } finally {
    Object.defineProperty(prototype, 'setItem', descriptor);
  }
});
