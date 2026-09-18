import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import type { Bridge, ResetPreview } from '../src/bridge';
import { installDom } from './lib/dom-harness';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
  act: true,
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

test('a delayed reset preview cannot populate another selected profile', async () => {
  const { ServersSection } = (await vite.ssrLoadModule(
    '/src/screens/servers-screen.tsx',
  )) as typeof import('../src/screens/servers-screen');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const { ToastProvider, ToastController } = (await vite.ssrLoadModule(
    '/kit/toasts.tsx',
  )) as typeof import('../kit/toasts');
  const { OverlayProvider } = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives');
  const original = mockBridge(FIXTURE);
  const first = FIXTURE.servers[0].id,
    second = FIXTURE.servers[1].id;
  const oldPreview = {
    ...(await original.describeReset(first)),
    resumables: [
      { kind: 'account-signup' as const, alias: 'old-preview-marker' },
    ],
  };
  const newPreview = {
    ...(await original.describeReset(second)),
    resumables: [
      { kind: 'account-signup' as const, alias: 'new-preview-marker' },
    ],
  };
  let finish!: (value: ResetPreview) => void;
  const old = new Promise<ResetPreview>((resolve) => {
    finish = resolve;
  });
  let requests = 0;
  const bridge: Bridge = {
    ...original,
    describeReset: (profile) => {
      if (profile === first) {
        requests++;
        return old;
      }
      return Promise.resolve(newPreview);
    },
    resetServer: async () => assert.fail('no reset is authorized by this test'),
  };
  const props = {
    snapshot: FIXTURE,
    bridge,
    profile: first,
    scene: 'servers-reset',
    onNavigate: () => {},
    onRefresh: async () => {},
    onError: () => {},
    onMutationError: async () => {},
  };
  const controller = new ToastController();
  const draw = () =>
    createElement(OverlayProvider, {
      backgroundRef: { current: document.getElementById('root') },
      portalRoot: document.getElementById('overlays')!,
      children: createElement(ToastProvider, {
        controller,
        children: createElement(ServersSection, props),
      }),
    });
  const view = ui.render(draw());
  await ui.waitFor(() => assert.equal(requests, 1));
  props.profile = second;
  view.rerender(draw());
  if (view.queryByRole('dialog') || view.queryByRole('alertdialog'))
    await view.findByText('new-preview-marker');
  await ui.act(async () => {
    finish(oldPreview);
  });
  assert.equal(Boolean(view.queryByText('old-preview-marker')), false);
});
