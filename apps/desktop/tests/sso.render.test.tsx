import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import { installDom } from './lib/dom-harness';
import { decodeSsoProgress } from '../src/sso-contract';
import type { SsoProgress } from '../src/sso-contract';
installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div>',
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
const progress: SsoProgress = {
  operationId: '1'.repeat(32),
  accountAlias: 'work',
  forLogin: true,
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
    createElement(SsoPanel, {
      bridge,
      profile: 'host',
      account: 'work',
      login: true,
      onComplete: () => {
        complete++;
      },
    }),
  );
  ui.fireEvent.click(r.getByText('Continue with organization'));
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
    createElement(SsoPanel, {
      bridge,
      profile: 'host',
      account: 'work',
      login: true,
      onComplete: () => {
        throw new Error('wrong account');
      },
    }),
  );
  ui.fireEvent.click(r.getByText('Continue with organization'));
  await ui.waitFor(() => assert.ok(r.getByRole('alert')));
  assert.equal(r.queryByText('Open sign-in browser'), null);
});
