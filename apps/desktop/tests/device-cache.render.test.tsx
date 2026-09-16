import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import { installDom } from './lib/dom-harness';
import type { Bridge } from '../src/bridge';

installDom({
  url: 'http://localhost/?state=people',
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
test.afterEach(() => ui.cleanup());
test.after(async () => vite.close());

test('Accounts and Devices share metadata across navigation; Refresh reloads it and card presence stays live', async () => {
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
  ui.render(createElement(App, { bridge }));
  await ui.waitFor(() => assert.equal(calls.enrollments, 1));
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
  await ui.waitFor(() => assert.equal(calls.cards, 1));
  assert.deepEqual(calls, { devices: 1, backups: 1, enrollments: 1, cards: 1 });
  assert.equal(document.body.textContent?.includes('Loading devices'), false);
  await ui.act(async () => {
    releaseCard([]);
  });
  await navigate('Accounts');
  await navigate('Devices');
  await ui.waitFor(() => assert.equal(calls.cards, 2));
  assert.equal(calls.devices, 1);
  const refresh = document.querySelector<HTMLButtonElement>(
    '.topbar button[aria-label="Refresh"]',
  );
  assert.ok(refresh);
  connected = [{ serial: 87654321 }];
  await ui.act(async () => {
    ui.fireEvent.click(refresh);
  });
  await ui.waitFor(() => assert.equal(calls.devices, 2));
  assert.equal(calls.backups, 2);
  assert.equal(calls.enrollments, 2);
  await ui.waitFor(() => {
    assert.equal(calls.cards, 3);
    assert.ok(document.body.textContent?.includes('87654321'));
  });
  connected = [];
  await ui.act(async () => {
    ui.fireEvent.click(refresh);
  });
  await ui.waitFor(() => {
    assert.equal(calls.cards, 4);
    assert.ok(
      document.body.textContent?.includes('No security key connected.'),
    );
  });
  assert.equal(document.body.textContent?.includes('87654321'), false);
});
