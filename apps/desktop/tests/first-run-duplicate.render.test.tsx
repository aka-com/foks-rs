import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import type { Bridge } from '../src/bridge';
import type { FirstRunExperienceProps } from '../src/screens/first-run-screen';
import type { FirstRunCheckpoint } from '../src/first-run-state';
import type { World } from '../src/model';
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

const OFFER = 'Use existing account';

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
  const absent = {
    ...complete,
    accounts: complete.accounts.filter(
      (account) => account.alias !== 'personal',
    ),
  };
  const checkpoint: FirstRunCheckpoint = {
    ...state.initialFirstRun('own', 'account'),
    managedLocal: true,
    serverAddress: 'localhost:4430',
    profile,
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
    absent,
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
        world: complete,
        location: { kind: 'first-run', path: 'own', step: saved.state },
        onNavigate: () => {},
        onRefreshWorld: async () => complete,
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

function counting(
  base: Bridge,
  code: string,
): {
  bridge: Bridge;
  calls: () => { creates: number; resumes: number; tracked: number };
} {
  let creates = 0;
  let resumes = 0;
  let tracked = 0;
  const refusal = {
    code,
    message: 'an account with this username already exists',
    retryable: false,
    ambiguous: false,
    fatal: false,
  };
  const bridge: Bridge = {
    ...base,
    createFirstRunAccount: async () => {
      creates++;
      throw refusal;
    },
    resumeFirstRunAccount: async () => {
      resumes++;
      throw refusal;
    },
    runFirstRunAccountOperation: async () => {
      tracked++;
      throw refusal;
    },
    listPendingOperations: async () => [],
  };
  return { bridge, calls: () => ({ creates, resumes, tracked }) };
}

async function refuse(
  h: Awaited<ReturnType<typeof harness>>,
  code: string,
  refreshed: World,
) {
  const { bridge, calls } = counting(h.bridge, code);
  const rendered = h.render(h.checkpoint, {
    bridge,
    world: h.complete,
    onRefreshWorld: async () => refreshed,
  });
  ui.fireEvent.change(rendered.view.getByPlaceholderText('yourname'), {
    target: { value: 'personal' },
  });
  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.view.getByRole('button', { name: 'Create my account' }),
    );
  });
  return { rendered, calls };
}

test('shows existing account option when username is already registered and confirmed by inventory', async () => {
  const h = await harness();
  const { rendered, calls } = await refuse(h, 'already-exists', h.complete);
  await rendered.view.findByRole('button', { name: OFFER });
  assert.equal(h.saved()?.state, 'account');
  assert.equal(h.saved()?.account, undefined);
  assert.equal(h.saved()?.provisioning, undefined);
  const before = calls();
  assert.equal(before.tracked, 1);
  await ui.act(async () => {
    ui.fireEvent.click(rendered.view.getByRole('button', { name: OFFER }));
  });
  assert.equal(h.saved()?.state, 'protect');
  assert.equal(h.saved()?.account?.alias, 'personal');
  assert.equal(h.saved()?.account?.username, 'rae');
  assert.deepEqual(calls(), before);
  assert.deepEqual(before, { creates: 0, resumes: 0, tracked: 1 });
});

test('does not show existing account option when alias is absent from inventory', async () => {
  const h = await harness();
  const { rendered } = await refuse(h, 'already-exists', h.absent);
  await rendered.view.findByText(
    'an account with this username already exists',
  );
  assert.equal(rendered.view.queryByRole('button', { name: OFFER }), null);
  assert.equal(h.saved()?.state, 'account');
  assert.equal(h.saved()?.account, undefined);
});

test('does not show existing account option when refreshed inventory is unavailable', async () => {
  const h = await harness();
  const { rendered } = await refuse(h, 'already-exists', h.unknown);
  await rendered.view.findByText(
    'an account with this username already exists',
  );
  assert.equal(rendered.view.queryByRole('button', { name: OFFER }), null);
  assert.equal(h.saved()?.state, 'account');
  assert.equal(h.saved()?.account, undefined);
});

test('clears existing account option when username is edited', async () => {
  const h = await harness();
  const { rendered } = await refuse(h, 'already-exists', h.complete);
  await rendered.view.findByRole('button', { name: OFFER });
  ui.fireEvent.change(rendered.view.getByPlaceholderText('yourname'), {
    target: { value: 'personal2' },
  });
  assert.equal(rendered.view.queryByRole('button', { name: OFFER }), null);
  assert.equal(h.saved()?.state, 'account');
  assert.equal(h.saved()?.account, undefined);
});

test('does not show existing account option when signup fails for other reasons', async () => {
  const h = await harness();
  const { rendered } = await refuse(h, 'invalid-request', h.complete);
  await rendered.view.findByText(
    'an account with this username already exists',
  );
  assert.equal(rendered.view.queryByRole('button', { name: OFFER }), null);
  assert.equal(h.saved()?.state, 'account');
  assert.equal(h.saved()?.account, undefined);
});
