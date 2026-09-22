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
      version: '0.3.0',
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
  await resumed.findByRole('button', { name: 'Check again' });
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
    name: 'Retry setup',
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
