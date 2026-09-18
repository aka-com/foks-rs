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
    servers: FIXTURE.servers.map((server) => ({ ...server, label })),
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
  return { snapshot, bridge, wrap, name: label ?? 'Loading...' };
}

for (const label of ['Local server alias', null]) {
  test(`account switcher uses ${label ?? 'Loading...'} without profile suffixes`, async () => {
    const h = await setup(label);
    const { Sidebar } = (await vite.ssrLoadModule(
      '/src/shell/sidebar.tsx',
    )) as typeof import('../src/shell/sidebar');
    const view = ui.render(
      h.wrap(
        createElement(Sidebar, {
          snapshot: h.snapshot,
          location: { kind: 'all' },
          onNavigate: () => {},
          onLock: () => {},
        }),
      ),
    );
    const button = view.container.querySelector<HTMLButtonElement>('.who');
    assert.ok(button);
    ui.fireEvent.click(button);
    const captions = document.querySelectorAll('.rail-account-menu .cap');
    assert.ok(captions.length > 0);
    for (const caption of captions)
      assert.equal(caption.textContent?.trim(), h.name);
    assert.equal(
      Boolean(document.querySelector('.rail-account-menu .cap .host')),
      false,
    );
  });

  test(`server page and breadcrumb use ${label ?? 'Loading...'} while retaining the Internal ID`, async () => {
    const h = await setup(label);
    const { ServersSection } = (await vite.ssrLoadModule(
      '/src/screens/servers-screen.tsx',
    )) as typeof import('../src/screens/servers-screen');
    const { crumbTrail } = (await vite.ssrLoadModule(
      '/src/shell/topbar.tsx',
    )) as typeof import('../src/shell/topbar');
    const profile = h.snapshot.servers[0].id;
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
    const names = list.container.querySelectorAll('.srow .t > b');
    assert.ok(names.length > 0);
    for (const name of names) assert.equal(name.textContent, h.name);
    list.unmount();
    const detail = ui.render(
      h.wrap(createElement(ServersSection, { ...props, profile })),
    );
    assert.equal(
      detail.container.querySelector('.shead b')?.textContent,
      h.name,
    );
    assert.doesNotMatch(
      detail.container.querySelector('.shead')?.textContent ?? '',
      /Profile /,
    );
    assert.ok(detail.getByText('Internal ID'));
    assert.equal(
      crumbTrail(
        { kind: 'settings', section: 'servers', profile },
        h.snapshot,
      ).at(-1),
      h.name,
    );
    assert.equal(
      crumbTrail({ kind: 'settings', section: 'servers', profile }).at(-1),
      'Loading...',
    );
  });
}
