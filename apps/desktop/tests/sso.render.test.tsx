import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactNode } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import { installDom } from './lib/dom-harness';
import { workflowScope } from './lib/workflow-scope';
import { decodeSsoProgress } from '../src/sso-contract';
import type { SsoProgress } from '../src/sso-contract';
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
test.afterEach(() => ui.cleanup());
test.after(async () => vite.close());

/** The sheet portals into the overlay root, so every panel needs a provider. */
async function overlay(children: ReactNode) {
  const { OverlayProvider } = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives');
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  return createElement(OverlayProvider, {
    backgroundRef: { current: null },
    portalRoot,
    children: await workflowScope(vite, children),
  });
}

const presentation = {
  title: 'Organization sign-in',
  subtitle: 'ada on Example',
  onClose: () => {},
};
const progress: SsoProgress = {
  operationId: '1'.repeat(32),
  accountAlias: 'work',
  purpose: 'reauthenticate',
  accountStatus: null,
  state: 'waiting',
  browserAvailable: true,
  expiresAtMs: 1700000000000,
  serviceAccess: false,
};
test('browser credentials and unknown fields are rejected at the JS boundary', () => {
  assert.deepEqual(decodeSsoProgress(progress), progress);
  for (const field of ['browserUrl', 'id_token', 'access_token', 'nonce'])
    assert.throws(() => decodeSsoProgress({ ...progress, [field]: 'secret' }));
  assert.throws(() => decodeSsoProgress({ ...progress, operationId: 'other' }));
  assert.throws(() => decodeSsoProgress({ ...progress, state: 'invented' }));
});
test('reopening a saved accepted signup uses status without replaying signup', async () => {
  const { SsoPanel } = (await vite.ssrLoadModule(
    '/src/components/sso-panel.tsx',
  )) as typeof import('../src/components/sso-panel');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  for (const state of ['complete', 'service-unavailable'] as const) {
    const actions: string[] = [];
    let completed = 0;
    const view = ui.render(
      await workflowScope(
        vite,
        createElement(SsoPanel, {
          bridge: {
            ...mockBridge(),
            sso: async (_profile, _alias, action) => {
              actions.push(action.action);
              return { ...progress, purpose: 'signup', state };
            },
          },
          profile: 'host',
          account: 'work',
          login: false,
          initialOperationId: progress.operationId!,
          onComplete: () => {
            completed++;
          },
        }),
      ),
    );
    await ui.waitFor(() => assert.equal(completed, 1));
    assert.deepEqual(actions, ['status']);
    view.unmount();
  }
});
test('consent, signed completion and usable access are distinct; no automatic operation replay', async () => {
  const { SsoPanel } = (await vite.ssrLoadModule(
    '/src/components/sso-panel.tsx',
  )) as typeof import('../src/components/sso-panel');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const actions: string[] = [];
  let opened = 0;
  let complete = 0;
  const bridge = {
    ...mockBridge(),
    sso: async (
      _p: string,
      _a: string,
      action: import('../src/sso-contract').SsoAction,
    ) => {
      actions.push(action.action);
      return {
        ...progress,
        state:
          action.action === 'poll'
            ? 'ready'
            : action.action === 'finish-login'
              ? 'complete'
              : 'waiting',
        browserAvailable: action.action === 'begin',
      } as SsoProgress;
    },
    openSsoBrowser: async () => {
      opened++;
      return { ok: true as const };
    },
  };
  const r = ui.render(
    await overlay(
      createElement(SsoPanel, {
        bridge,
        profile: 'host',
        account: 'work',
        login: true,
        presentation,
        onComplete: () => {
          complete++;
        },
      }),
    ),
  );
  ui.fireEvent.click(r.getByText('Sign in'));
  await ui.waitFor(() => assert.ok(r.getByText('Open sign-in browser')));
  assert.equal(opened, 0);
  ui.fireEvent.click(r.getByText('Open sign-in browser'));
  await ui.waitFor(() => assert.equal(opened, 1));
  ui.fireEvent.click(r.getByText('Check sign-in'));
  await ui.waitFor(() => assert.ok(r.getByText('Finish sign-in')));
  assert.equal(complete, 0);
  ui.fireEvent.click(r.getByText('Finish sign-in'));
  await ui.waitFor(() =>
    assert.ok(r.getByText('The signed account operation was accepted.')),
  );
  assert.equal(complete, 0);
  assert.deepEqual(actions, ['begin', 'poll', 'finish-login']);
});
test('a response for another account never updates the displayed flow', async () => {
  const { SsoPanel } = (await vite.ssrLoadModule(
    '/src/components/sso-panel.tsx',
  )) as typeof import('../src/components/sso-panel');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const bridge = {
    ...mockBridge(),
    sso: async () => ({ ...progress, accountAlias: 'other' }),
  };
  const r = ui.render(
    await overlay(
      createElement(SsoPanel, {
        bridge,
        profile: 'host',
        account: 'work',
        login: true,
        presentation,
        onComplete: () => {
          throw new Error('wrong account');
        },
      }),
    ),
  );
  ui.fireEvent.click(r.getByText('Sign in'));
  await ui.waitFor(() => assert.ok(r.getByRole('alert')));
  assert.equal(r.queryByText('Open sign-in browser'), null);
});

test('account linkage status chooses explicit first-link action and preserves lockout messaging', async () => {
  const { SsoPanel } = (await vite.ssrLoadModule(
    '/src/components/sso-panel.tsx',
  )) as typeof import('../src/components/sso-panel');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const actions: import('../src/sso-contract').SsoAction[] = [];
  const bridge = {
    ...mockBridge(),
    sso: async (
      _p: string,
      _a: string,
      action: import('../src/sso-contract').SsoAction,
    ): Promise<SsoProgress> => {
      actions.push(action);
      if (action.action === 'account-status')
        return {
          ...progress,
          operationId: null,
          purpose: 'link-existing',
          state: 'locked-out',
          browserAvailable: false,
          accountStatus: {
            state: 'locked-out',
            rolloutMode: 2,
            providerFence: 0,
            issuer: 'https://identity.example',
            authorizationEpoch: 1,
            authorizationGeneration: 0,
          },
        };
      return { ...progress, purpose: 'link-existing' };
    },
  };
  const r = ui.render(
    await overlay(
      createElement(SsoPanel, {
        bridge,
        profile: 'host',
        account: 'work',
        login: true,
        presentation,
        onComplete: () => {},
      }),
    ),
  );
  ui.fireEvent.click(r.getByText('Check linkage'));
  await ui.waitFor(() =>
    assert.ok(r.getByText(/Organization sign-in is enforced/)),
  );
  ui.fireEvent.click(r.getByText('Link existing account'));
  await ui.waitFor(() => assert.ok(r.getByText('Open sign-in browser')));
  assert.deepEqual(
    actions.map((a) => a.action),
    ['account-status', 'begin'],
  );
  assert.equal(
    actions[1].action === 'begin' && actions[1].purpose,
    'link-existing',
  );
});

test('SSO login hides the PIN behind Hardware key and clears it when disabled', async () => {
  const { SsoPanel } = (await vite.ssrLoadModule(
    '/src/components/sso-panel.tsx',
  )) as typeof import('../src/components/sso-panel');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  ui.render(
    await overlay(
      createElement(SsoPanel, {
        bridge: mockBridge(),
        profile: 'host',
        account: 'work',
        login: true,
        presentation,
        onComplete: () => {},
      }),
    ),
  );
  const pinLabel = 'Security key PIN (enrolled keys only)';
  assert.equal(ui.screen.queryByLabelText(pinLabel), null);
  const toggle = ui.screen.getByRole('checkbox', { name: 'Hardware key' });
  ui.fireEvent.click(toggle);
  ui.fireEvent.change(ui.screen.getByLabelText(pinLabel), {
    target: { value: '123456' },
  });
  ui.fireEvent.click(toggle);
  assert.equal(ui.screen.queryByLabelText(pinLabel), null);
  ui.fireEvent.click(toggle);
  assert.equal(ui.screen.getByLabelText<HTMLInputElement>(pinLabel).value, '');
});
