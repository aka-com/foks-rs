import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, type ReactNode } from 'react';
import { installDom } from './lib/dom-harness';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
  act: true,
});
let ui: typeof import('@testing-library/react');
test.before(async () => {
  ui = await import('@testing-library/react');
});
test.afterEach(() => ui.cleanup());

async function setup(label: string | null) {
  const { FIXTURE } = await import('../src/fixture');
  const { mockBridge } = await import('../src/mock-bridge');
  const { ToastProvider, ToastController } = await import('../kit/toasts');
  const { OverlayProvider } = await import('../kit/overlay-primitives');
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
  return { snapshot, bridge, wrap, name: label ?? 'Loading...' };
}

for (const label of ['Local server alias', null]) {
  test(`server page and breadcrumb use ${label ?? 'Loading...'} while retaining the Internal ID`, async () => {
    const h = await setup(label);
    const { ServersSection } = await import('../src/screens/servers-screen');
    const { crumbTrail } = await import('../src/shell/topbar');
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
    const names = list.container.querySelectorAll('.srow .t > b');
    assert.ok(names.length > 0);
    for (const name of names) assert.equal(name.textContent, h.name);
    list.unmount();
    const detail = ui.render(
      h.wrap(createElement(ServersSection, { ...props, profile })),
    );
    assert.equal(detail.getByRole('heading', { level: 1 }).textContent, h.name);
    assert.equal(detail.container.querySelector('.shead'), null);
    assert.ok(detail.getByText('Internal ID'));
    assert.equal(
      crumbTrail(
        { kind: 'settings', section: 'account', profile },
        h.snapshot,
      ).at(-1),
      h.name,
    );
    assert.equal(
      crumbTrail({ kind: 'settings', section: 'account', profile }).at(-1),
      'Loading...',
    );
  });
}
