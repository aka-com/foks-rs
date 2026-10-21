import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, type ReactNode } from 'react';
import { createServer, type ViteDevServer } from 'vite';
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

async function setup(label: string | null) {
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
  const snapshot = {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server) => ({
      ...server,
      displayLabel: label,
    })),
  };
  const bridge = mockBridge(snapshot);
  const wrap = (children: ReactNode) =>
    createElement(OverlayProvider, {
      backgroundRef: { current: null },
      portalRoot: document.getElementById('overlays')!,
      children: createElement(ToastProvider, {
        controller: new ToastController(),
        children,
      }),
    });
  return { snapshot, bridge, wrap, name: label };
}

for (const label of ['Local server alias', null]) {
  test(`server page, breadcrumb and list row name the server by ${label ?? 'its address'} while retaining the Internal ID`, async () => {
    const h = await setup(label);
    const { ServersSection } = (await vite.ssrLoadModule(
      '/src/screens/servers-screen.tsx',
    )) as typeof import('../src/screens/servers-screen');
    const { crumbTrail } = (await vite.ssrLoadModule(
      '/src/shell/topbar.tsx',
    )) as typeof import('../src/shell/topbar');
    const profile = h.snapshot.servers[0].profileName;
    const props = {
      snapshot: h.snapshot,
      bridge: h.bridge,
      scene: '',
      onNavigate: () => {},
      onRefresh: async () => {},
      onError: () => {},
      onMutationError: async () => {},
    };
    const list = ui.render(h.wrap(createElement(ServersSection, props)));
    // A server with no display label (it has none before its first check)
    // is named by its address on its row, its page and in the crumbs.
    const names = [...list.container.querySelectorAll('.srow .t > b')];
    assert.ok(names.length > 0);
    // The list puts servers needing attention first, so compare as sets.
    assert.deepEqual(
      names.map((name) => name.textContent).sort(),
      h.snapshot.servers
        .map((server) => label ?? server.configuredEndpoint)
        .sort(),
    );
    list.unmount();
    const detail = ui.render(
      h.wrap(createElement(ServersSection, { ...props, profile })),
    );
    const name = h.name ?? h.snapshot.servers[0].configuredEndpoint;
    assert.equal(detail.getByRole('heading', { level: 1 }).textContent, name);
    assert.equal(detail.container.querySelector('.shead'), null);
    assert.ok(detail.getByText('Internal ID'));
    assert.equal(
      crumbTrail(
        { kind: 'settings', section: 'account', profile },
        h.snapshot,
      ).at(-1),
      name,
    );
    // Before the snapshot names the server, the crumb says it is loading.
    assert.equal(
      crumbTrail({ kind: 'settings', section: 'account', profile }).at(-1),
      'Loading…',
    );
  });
}
