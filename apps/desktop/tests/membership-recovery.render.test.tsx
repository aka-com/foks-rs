import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import type { Bridge, PendingOperation } from '../src/bridge';
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
  window.history.replaceState(null, '', '/');
});
test.after(async () => {
  await vite.close();
});

for (const kind of ['team-member-addition', 'team-member-edit'] as const) {
  test(`exposes and completes ${kind} after an interrupted change without reopening settings`, async () => {
    const { GroupSettingsScreen } = (await vite.ssrLoadModule(
      '/src/screens/groups-screen.tsx',
    )) as typeof import('../src/screens/groups-screen');
    const { ToastProvider, ToastController } = (await vite.ssrLoadModule(
      '/kit/toasts.tsx',
    )) as typeof import('../kit/toasts');
    const { OverlayProvider } = (await vite.ssrLoadModule(
      '/kit/overlay-primitives.tsx',
    )) as typeof import('../kit/overlay-primitives');
    const { FIXTURE } = (await vite.ssrLoadModule(
      '/src/fixture.ts',
    )) as typeof import('../src/fixture');
    const { mockBridge } = (await vite.ssrLoadModule(
      '/src/mock-bridge.ts',
    )) as typeof import('../src/mock-bridge');
    const addition = kind === 'team-member-addition';
    window.history.replaceState(
      null,
      '',
      addition ? '/?state=add' : '/?state=demote',
    );
    const store = FIXTURE.stores.find((entry) => entry.id === 'team:eng');
    assert.ok(store?.kind === 'team');
    const portalRoot = document.getElementById('overlays');
    assert.ok(portalRoot);
    let pending: PendingOperation[] = [];
    let reads = 0;
    let reconciled = false;
    let resumed = false;
    const interrupted = async () => {
      pending = [
        {
          kind,
          alias: store.alias,
          ...(addition ? { target: 'jules.park' } : {}),
        },
      ];
      throw {
        code: 'ambiguous',
        message: 'Interrupted',
        retryable: false,
        ambiguous: true,
        fatal: false,
      };
    };
    const bridge: Bridge = {
      ...mockBridge(FIXTURE),
      listPendingOperations: async () => {
        reads++;
        return pending;
      },
      addGroupMember: interrupted,
      demoteGroupMember: interrupted,
      resumeGroupMemberAddition: async (request) => {
        assert.equal(request.storeId, store.id);
        assert.equal(request.username, 'jules.park');
        resumed = true;
        pending = [];
        return { applied: true };
      },
      resumeGroupMemberEdit: async (storeId) => {
        assert.equal(storeId, store.id);
        resumed = true;
        pending = [];
        return { applied: true };
      },
    };
    const rendered = ui.render(
      createElement(OverlayProvider, {
        backgroundRef: { current: null },
        portalRoot,
        children: createElement(ToastProvider, {
          controller: new ToastController(),
          children: createElement(GroupSettingsScreen, {
            world: FIXTURE,
            bridge,
            location: { kind: 'group-settings', ref: store.id, tab: 'people' },
            onNavigate: () => {},
            onApplied: async () => {},
            onError: (error: unknown) => {
              throw error;
            },
            onMutationError: async () => {
              reconciled = true;
            },
          }),
        }),
      }),
    );
    await ui.act(async () => {});
    assert.equal(reads, 1);
    assert.equal(
      rendered.queryByText('Finish a pending membership change'),
      null,
    );
    await ui.act(async () => {
      ui.fireEvent.click(
        rendered.getByRole('button', {
          name: addition ? 'Add jules.park' : 'Change role',
        }),
      );
    });
    assert.ok(reconciled);
    // A failed sheet stays open. Close it to use the newly displayed Resume action.
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Cancel' }));
    const resumeName = addition
      ? 'Resume adding jules.park'
      : 'Resume the role change';
    await ui.act(async () => {
      ui.fireEvent.click(rendered.getByRole('button', { name: resumeName }));
    });
    assert.ok(resumed);
    assert.equal(
      rendered.queryByText('Finish a pending membership change'),
      null,
    );
  });
}
