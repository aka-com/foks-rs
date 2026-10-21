import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import type { Bridge } from '../src/bridge';
import type { AgentSnapshot } from '../src/model/types';
import { installDom } from './lib/dom-harness';

const dom = installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
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

test.after(async () => {
  await vite.close();
  dom.window.close();
});

function resetLocation(): void {
  window.history.replaceState(null, '', '/');
}

async function unclaimedSnapshot(): Promise<AgentSnapshot> {
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  return {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server) =>
      server.profileName === 'personal'
        ? { ...server, trust: { status: 'unprobed' as const } }
        : server,
    ),
    stores: [],
    accounts: [],
    items: [],
    parties: [],
  };
}

test('first-run resume reports an unreachable managed local server', async () => {
  const { App } = (await vite.ssrLoadModule(
    '/src/app-root.tsx',
  )) as typeof import('../src/app-root');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const { encodeFirstRunCheckpoint, FIRST_RUN_CHECKPOINT_KEY } =
    (await vite.ssrLoadModule(
      '/src/first-run-state.ts',
    )) as typeof import('../src/first-run-state');

  const base = mockBridge(await unclaimedSnapshot());
  const describedProfiles: string[] = [];
  const bridge: Bridge = {
    ...base,
    appInfo: async () => ({
      version: '0.4.0',
      agentSocket: '/private/foks/agent.sock',
      managedProfile: 'personal',
    }),
    describeServerStatus: async (profile) => {
      describedProfiles.push(profile);
      return base.describeServerStatus(profile);
    },
  };

  window.localStorage.removeItem(FIRST_RUN_CHECKPOINT_KEY);
  resetLocation();
  const fresh = ui.render(createElement(App, { bridge }));
  await fresh.findByRole('heading', { name: 'How are you joining?' });
  assert.equal(describedProfiles.length, 0);
  fresh.unmount();

  window.localStorage.setItem(
    FIRST_RUN_CHECKPOINT_KEY,
    encodeFirstRunCheckpoint({
      version: 3,
      path: 'own',
      state: 'local',
      managedLocal: true,
      passphraseSet: false,
      backupCommitted: false,
      protectSkipped: false,
      added: false,
      returning: false,
    }),
  );
  resetLocation();
  const resumed = ui.render(createElement(App, { bridge }));
  await resumed.findByRole('heading', { name: 'Set up FOKS' });
  await resumed.findByText('Local server unavailable');
  await resumed.findByRole('button', { name: 'Check Again' });
  assert.deepEqual(describedProfiles, ['personal']);
  resumed.unmount();
  window.localStorage.removeItem(FIRST_RUN_CHECKPOINT_KEY);
});

test('first-run retry reconnects the agent transport after a connection loss', async () => {
  const { App } = (await vite.ssrLoadModule(
    '/src/app-root.tsx',
  )) as typeof import('../src/app-root');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const { FIRST_RUN_CHECKPOINT_KEY } = (await vite.ssrLoadModule(
    '/src/first-run-state.ts',
  )) as typeof import('../src/first-run-state');
  window.localStorage.removeItem(FIRST_RUN_CHECKPOINT_KEY);
  resetLocation();

  const base = mockBridge(await unclaimedSnapshot());
  let statusCalls = 0;
  let retryCalls = 0;
  let statusCallsAtRetry = -1;
  let lossReported = false;
  const bridge: Bridge = {
    ...base,
    agentStatus: async () => {
      statusCalls++;
      return { state: 'ready' };
    },
    takeAgentConnectionLoss: async () => {
      if (lossReported) return null;
      lossReported = true;
      return 'The background service stopped responding.';
    },
    retryAgentConnection: async () => {
      retryCalls++;
      statusCallsAtRetry = statusCalls;
      return { state: 'ready' };
    },
  };

  const rendered = ui.render(createElement(App, { bridge }));
  assert.ok(
    await ui.screen.findByRole('alertdialog', {
      name: 'Connection to background service lost',
    }),
  );
  const retry = await rendered.findByRole('button', {
    name: 'Retry Setup',
    hidden: true,
  });
  const statusCallsBeforeRetry = statusCalls;
  await ui.act(async () => {
    retry.click();
    await Promise.resolve();
    await Promise.resolve();
  });
  await ui.waitFor(() => assert.equal(retryCalls, 1));
  assert.equal(
    statusCallsAtRetry,
    statusCallsBeforeRetry,
    'the reconnect used retryAgentConnection rather than agentStatus',
  );
  rendered.unmount();
});

test('the Added step is a page in the panel, not a list column beside empty details', async () => {
  const { App } = (await vite.ssrLoadModule(
    '/src/app-root.tsx',
  )) as typeof import('../src/app-root');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { FIRST_RUN_CHECKPOINT_KEY } = (await vite.ssrLoadModule(
    '/src/first-run-state.ts',
  )) as typeof import('../src/first-run-state');
  window.localStorage.removeItem(FIRST_RUN_CHECKPOINT_KEY);
  window.history.replaceState(null, '', '/?state=added');
  const rendered = ui.render(
    createElement(App, { bridge: mockBridge(FIXTURE) }),
  );
  const open = await rendered.findByRole('button', { name: 'Open Team' });

  // The step used to be laid out in the Files list column (`with-details`),
  // which clipped its notice's actions at every window size.
  const app = document.querySelector('.app');
  assert.ok(app);
  assert.equal(app.classList.contains('with-details'), false);
  assert.equal(document.querySelector('.app > .details'), null);
  const main = open.closest('main.first-run-main');
  assert.ok(main, 'Open team is in the step’s own panel');
  // The panel opens on its bar, which names the team; the page does not
  // repeat it as a heading.
  const bar = main.querySelector(':scope > .topbar.bar');
  assert.ok(bar);
  assert.equal(bar.querySelector('.crumbs .cur')?.textContent, 'Engineering');
  assert.equal(main.querySelector('.path'), null);
  rendered.unmount();
  resetLocation();
});
