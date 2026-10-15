import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactNode } from 'react';
import type { AgentSnapshot } from '../src/model/types';
import { installDom } from './lib/dom-harness';
import { workflowScope } from './lib/workflow-scope';

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

async function overlay(children: ReactNode, snapshot: AgentSnapshot) {
  const { OverlayProvider } = await import('../kit/overlay-primitives');
  return createElement(OverlayProvider, {
    backgroundRef: { current: null },
    portalRoot: document.getElementById('overlays')!,
    children: await workflowScope(children, snapshot),
  });
}

async function fixture() {
  const { FIXTURE } = await import('../src/fixture');
  return structuredClone(FIXTURE);
}

test('a queued workflow uses the latest snapshot instead of launch permission', async () => {
  const { useWorkflowAccess } = await import('../src/workflow-context');
  const current = await fixture();
  let dispatch: () => Promise<void> = async () => {};
  let calls = 0;
  function Launcher({ snapshot }: { snapshot: AgentSnapshot }) {
    const access = useWorkflowAccess(snapshot);
    dispatch = () =>
      access.run(
        'backup-revoke',
        { profile: 'personal', account: 'personal' },
        async () => {
          calls++;
        },
      );
    return null;
  }
  const view = ui.render(createElement(Launcher, { snapshot: current }));
  const queued = dispatch;
  const changed = structuredClone(current);
  changed.servers[0].compatibility = {
    status: 'required',
    expiresAt: Math.floor(Date.now() / 1000) + 600,
    capabilities: ['device-administration'],
  };
  view.rerender(createElement(Launcher, { snapshot: changed }));
  await assert.rejects(queued(), /recovery/);
  assert.equal(calls, 0);
});

test('local aliases save while remote operations are unavailable', async () => {
  const { LocalAliasPanel } =
    await import('../src/components/local-alias-panel');
  const { mockBridge } = await import('../src/mock-bridge');
  const current = await fixture();
  current.servers[0].compatibility = { status: 'required-unavailable' };
  let saved = '';
  const bridge = {
    ...mockBridge(current),
    setLocalAccountAlias: async (store: string, alias: string) => {
      saved = alias;
      return { store, alias };
    },
  };
  ui.render(
    await overlay(
      createElement(LocalAliasPanel, {
        bridge,
        store: 'acct:personal',
        alias: 'Personal',
        presentation: { title: 'Change local alias', onClose() {} },
        onComplete: async () => {},
      }),
      current,
    ),
  );
  ui.fireEvent.change(ui.screen.getByRole('textbox'), {
    target: { value: 'Local name' },
  });
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Save' }));
  await ui.waitFor(() => assert.equal(saved, 'Local name'));
});

test('web admin needs user-sync, not device administration, and rechecks submit', async () => {
  const { AdminPanel } = await import('../src/components/admin-panel');
  const { mockBridge } = await import('../src/mock-bridge');
  const current = await fixture();
  current.servers[0].compatibility = {
    status: 'required',
    expiresAt: Math.floor(Date.now() / 1000) + 600,
    capabilities: ['user-sync'],
  };
  let opened = 0;
  const bridge = {
    ...mockBridge(current),
    openWebAdmin: async () => {
      opened++;
      return { ok: true as const };
    },
  };
  const panel = createElement(AdminPanel, {
    bridge,
    profile: 'personal',
    account: 'personal',
    presentation: { title: 'Web admin', onClose() {} },
  });
  const view = ui.render(await overlay(panel, current));
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Open admin panel' }),
  );
  await ui.waitFor(() => assert.equal(opened, 1));
  const denied = structuredClone(current);
  denied.servers[0].compatibility = { status: 'required-unavailable' };
  view.rerender(await overlay(panel, denied));
  assert.equal(
    ui.screen
      .getByRole('button', {
        name: 'Open admin panel',
      })
      .hasAttribute('disabled'),
    true,
  );
  assert.equal(opened, 1);
});

test('metadata dispatch checks current account capability before either device or backup reads', async () => {
  const { DeviceCache } = await import('../src/data/device-cache');
  const { mockBridge } = await import('../src/mock-bridge');
  const current = await fixture();
  let reads = 0;
  const cache = new DeviceCache({
    ...mockBridge(current),
    listAccountDevices: async () => {
      reads++;
      return [];
    },
    listBackupEnrollments: async () => {
      reads++;
      return [];
    },
  });
  cache.snapshot = () => current;
  const query = cache.account('personal', 'acct:personal');
  current.servers[0].compatibility = {
    status: 'required',
    expiresAt: Math.floor(Date.now() / 1000) + 600,
    capabilities: ['recovery'],
  };
  await assert.rejects(query.load(), /device-administration/);
  assert.equal(reads, 0);
  assert.equal(query.getSnapshot().data, undefined);
});

test('shared metadata preflight keeps its root snapshot source after a view leaves', async () => {
  const module = await import('../src/data/device-cache');
  const { mockBridge } = await import('../src/mock-bridge');
  const initial = await fixture();
  let current = initial,
    calls = 0;
  const bridge = {
    ...mockBridge(initial),
    listAccountDevices: async () => {
      calls++;
      return [];
    },
  };
  const cache = new module.DeviceCache(bridge);
  cache.snapshot = () => current;
  function View() {
    module.useDeviceQueries(bridge, initial);
    return null;
  }
  const view = ui.render(
    createElement(module.DeviceCacheContext.Provider, {
      value: cache,
      children: createElement(View),
    }),
  );
  view.unmount();
  current = structuredClone(initial);
  current.servers[0].compatibility = { status: 'required-unavailable' };
  await assert.rejects(
    cache.account('personal', 'acct:personal').load(),
    /check-in/,
  );
  assert.equal(calls, 0);
});

test('device rendering never scans hardware or prompts for credentials', async () => {
  const { DevicesScreen } = await import('../src/screens/devices-screen');
  const { mockBridge } = await import('../src/mock-bridge');
  const { ToastProvider, ToastController } = await import('../kit/toasts');
  const current = await fixture();
  let scans = 0;
  let hardwareCommands = 0;
  const bridge = {
    ...mockBridge(current),
    listYubiCards: async () => {
      scans++;
      return [];
    },
    runYubi: async () => {
      hardwareCommands++;
      return {};
    },
  };
  ui.render(
    await overlay(
      createElement(ToastProvider, {
        controller: new ToastController(),
        children: createElement(DevicesScreen, {
          snapshot: current,
          bridge,
          location: { kind: 'devices', store: 'acct:personal' },
          scene: '',
          onNavigate() {},
          onRefresh: async () => {},
          onRefreshSnapshot: async () => current,
          onError(error) {
            throw error;
          },
          onMutationError: async () => {},
        }),
      }),
      current,
    ),
  );
  await ui.act(async () => {
    await Promise.resolve();
  });
  assert.equal(scans, 0);
  assert.equal(hardwareCommands, 0);
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Refresh hardware keys' }),
  );
  await ui.waitFor(() => assert.equal(scans, 1));
  assert.equal(hardwareCommands, 0);
});
