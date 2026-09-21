import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import { installDom } from './lib/dom-harness';
import type { Bridge } from '../src/bridge';

installDom({
  url: 'http://localhost/?state=settings',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
  act: true,
});
let vite: ViteDevServer;
let ui: typeof import('@testing-library/react');
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
  Reflect.deleteProperty(document, 'hidden');
});
test.after(async () => vite.close());

test('Account and Devices share metadata across navigation; Refresh reloads it and card presence stays live', async () => {
  Object.defineProperty(document, 'hidden', {
    configurable: true,
    value: false,
  });
  const { App } = (await vite.ssrLoadModule(
    '/src/app-root.tsx',
  )) as typeof import('../src/app-root');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const base = mockBridge(FIXTURE);
  let releaseCard!: (cards: { serial: number }[]) => void;
  let connected: { serial: number }[] = [];
  const calls = { devices: 0, backups: 0, enrollments: 0, cards: 0 };
  const bridge: Bridge = {
    ...base,
    listAccountDevices: async (store) => {
      calls.devices++;
      return base.listAccountDevices(store);
    },
    listBackupEnrollments: async (store) => {
      calls.backups++;
      return base.listBackupEnrollments(store);
    },
    listYubiAccounts: async (profile) => {
      calls.enrollments++;
      return base.listYubiAccounts(profile);
    },
    listYubiCards: async () => {
      calls.cards++;
      if (calls.cards === 1)
        return new Promise((resolve) => {
          releaseCard = resolve;
        });
      return connected;
    },
  };
  const { deviceAlertRegistry } = (await vite.ssrLoadModule(
    '/src/screens/device-alert.ts',
  )) as typeof import('../src/screens/device-alert');
  ui.render(createElement(App, { bridge }));
  // The shell preloads every eligible account before visiting Devices.
  await ui.waitFor(() => {
    const keys = deviceAlertRegistry(bridge).getSnapshot().paperKeys;
    assert.equal(keys.get('acct:work'), false);
    assert.equal(keys.get('acct:personal'), true);
  });
  const initial = { ...calls };
  assert.deepEqual(initial, {
    devices: FIXTURE.accounts.length,
    backups: FIXTURE.accounts.length,
    enrollments: new Set(FIXTURE.accounts.map((account) => account.server))
      .size,
    cards: 0,
  });
  const navigate = async (name: string) => {
    const tab = [
      ...document.querySelectorAll<HTMLButtonElement>('.rail-tabs .nav'),
    ].find((node) => node.querySelector('.t')?.textContent === name);
    assert.ok(tab);
    await ui.act(async () => {
      ui.fireEvent.click(tab);
    });
  };
  await navigate('Devices');
  assert.equal(calls.cards, 0);
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Refresh hardware keys' }),
  );
  await ui.waitFor(() => assert.equal(calls.cards, 1));
  assert.deepEqual(calls, { ...initial, cards: 1 });
  assert.equal(document.body.textContent?.includes('Loading devices'), false);
  await ui.act(async () => {
    releaseCard([]);
  });
  await navigate('Settings');
  await navigate('Devices');
  assert.equal(calls.cards, 1);
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Refresh hardware keys' }),
  );
  await ui.waitFor(() => assert.equal(calls.cards, 2));
  assert.equal(calls.devices, initial.devices);
  const refresh = document.querySelector<HTMLButtonElement>(
    '.topbar button[aria-label="Refresh"]',
  );
  assert.ok(refresh);
  // "primary key" is the fixture's one complete enrollment on this account,
  // matched to card serial 20993145; its own page is where card presence is
  // now observable, in whether PIN status can run.
  const openPrimary = await ui.waitFor(() => {
    const button = document.querySelector<HTMLButtonElement>(
      'button[aria-label="Open primary key"]',
    );
    assert.ok(button);
    return button;
  });
  await ui.act(async () => {
    ui.fireEvent.click(openPrimary);
  });
  const pinStatus = await ui.waitFor(() => {
    const button = [
      ...document.querySelectorAll<HTMLButtonElement>('button'),
    ].find((node) => node.textContent === 'PIN status');
    assert.ok(button);
    return button;
  });
  assert.equal(pinStatus.hasAttribute('disabled'), true);
  connected = [{ serial: 20993145 }];
  await ui.act(async () => {
    ui.fireEvent.click(refresh);
  });
  await ui.waitFor(() => assert.equal(calls.devices, initial.devices * 2));
  assert.equal(calls.backups, initial.backups * 2);
  assert.equal(calls.enrollments, initial.enrollments * 2);
  assert.equal(calls.cards, 2);
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Refresh hardware keys' }),
  );
  await ui.waitFor(() => {
    assert.equal(calls.cards, 3);
    assert.equal(pinStatus.hasAttribute('disabled'), false);
  });
  connected = [];
  await ui.act(async () => {
    ui.fireEvent.click(refresh);
  });
  await ui.waitFor(() => assert.equal(calls.devices, initial.devices * 3));
  assert.equal(calls.cards, 3);
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Refresh hardware keys' }),
  );
  await ui.waitFor(() => {
    assert.equal(calls.cards, 4);
    assert.equal(pinStatus.hasAttribute('disabled'), true);
    assert.equal(pinStatus.getAttribute('title'), 'No security key connected.');
    // A scan that finds nothing looks identical to one that changed nothing,
    // so the empty result has to announce itself.
    assert.equal(
      document.body.textContent?.includes('No security keys found'),
      true,
    );
  });
});
