/**
 * Verifies that blocking modal overlays display error details and provide exit
 * actions.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import type { Bridge, MaintenanceSnapshot } from '../src/bridge';
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

test.afterEach(() => {
  ui.cleanup();
  window.localStorage.clear();
  window.history.replaceState(null, '', '/');
});

test.after(async () => {
  await vite.close();
  dom.window.close();
});

async function modules() {
  const { App } = (await vite.ssrLoadModule(
    '/src/app-root.tsx',
  )) as typeof import('../src/app-root');
  const { WriteOverlay } = (await vite.ssrLoadModule(
    '/src/screens/write-workflows.tsx',
  )) as typeof import('../src/screens/write-workflows');
  const { OverlayProvider } = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  return { App, WriteOverlay, OverlayProvider, FIXTURE, mockBridge };
}

async function agentLostOverlay(options: {
  message?: string;
  onRetryAgent: () => Promise<void>;
  quit?: () => Promise<void>;
}) {
  const { WriteOverlay, OverlayProvider, FIXTURE, mockBridge } =
    await modules();
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    quitApp: options.quit ?? (async () => {}),
  };
  const rendered = ui.render(
    createElement(OverlayProvider, {
      backgroundRef: { current: null },
      portalRoot,
      children: createElement(WriteOverlay, {
        snapshot: FIXTURE,
        bridge,
        workflow: { kind: 'agent-lost', message: options.message },
        setWorkflow: () => {},
        onApplied: async () => {},
        onError: () => {
          throw new Error('the dialog must not report through onError');
        },
        onMutationError: async () => {},
        onRetryAgent: options.onRetryAgent,
        onRefreshConflict: async () => {},
        onDiscardConflict: () => {},
        onOpenExisting: async () => {},
      }),
    }),
  );
  return { rendered, bridge };
}

async function startupOverlay(
  snapshot: MaintenanceSnapshot,
  overrides: Partial<Bridge> = {},
) {
  const { App, FIXTURE, mockBridge } = await modules();
  const base = mockBridge(FIXTURE);
  const bridge: Bridge = {
    ...base,
    native: true,
    clientStateMaintenanceStatus: async () => snapshot,
    onMaintenanceStatus: async () => () => {},
    ...overrides,
  };
  const rendered = ui.render(createElement(App, { bridge }));
  await ui.waitFor(() => {
    assert.ok(document.querySelector('.app-lock-card'));
  });
  return { rendered, bridge };
}

async function shellOverlay(
  snapshot: MaintenanceSnapshot,
  overrides: Partial<Bridge> = {},
) {
  const { App, FIXTURE, mockBridge } = await modules();
  const base = mockBridge(FIXTURE);
  const listeners = new Set<(value: MaintenanceSnapshot) => void>();
  let current: MaintenanceSnapshot = {
    state: 'idle',
    generation: 0,
    revision: 0,
  };
  const bridge: Bridge = {
    ...base,
    native: true,
    clientStateMaintenanceStatus: async () => current,
    onMaintenanceStatus: async (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    ...overrides,
  };
  const rendered = ui.render(createElement(App, { bridge }));
  await ui.waitFor(() => {
    assert.ok(document.querySelector('.app'));
  });
  current = snapshot;
  await ui.act(async () => {
    for (const listener of listeners) listener(snapshot);
    await Promise.resolve();
  });
  await ui.waitFor(() => {
    assert.ok(document.querySelector('.stopwrap'));
  });
  return { rendered, bridge };
}

test('the agent-lost dialog shows the reason it was given', async () => {
  const { rendered } = await agentLostOverlay({
    message: 'The agent socket closed while reading the catalog.',
    onRetryAgent: async () => {},
  });
  const dialog = document.querySelector('.stopwrap');
  assert.ok(dialog);
  assert.match(
    dialog.textContent ?? '',
    /The agent socket closed while reading the catalog\./,
  );
  rendered.unmount();
});

test('a failed agent-lost retry is reported inside the dialog', async () => {
  const { rendered } = await agentLostOverlay({
    message: 'The agent socket closed while reading the catalog.',
    onRetryAgent: async () => {
      throw new Error('The background service did not restart.');
    },
  });
  const retry = await rendered.findByRole('button', { name: 'Retry' });
  await ui.act(async () => {
    retry.click();
    await Promise.resolve();
  });
  await ui.waitFor(() => {
    assert.match(
      document.querySelector('.stopwrap')?.textContent ?? '',
      /The background service did not restart\./,
    );
  });
  assert.ok(document.querySelector('.stopwrap [role="alert"]'));
  rendered.unmount();
});

test('the agent-lost dialog quits through the bridge', async () => {
  let quits = 0;
  const { rendered } = await agentLostOverlay({
    onRetryAgent: async () => {},
    quit: async () => {
      quits++;
    },
  });
  const quit = await rendered.findByRole('button', { name: 'Quit FOKS' });
  await ui.act(async () => {
    quit.click();
    await Promise.resolve();
  });
  assert.equal(quits, 1);
  rendered.unmount();
});

const restartRequired: MaintenanceSnapshot = {
  state: 'complete',
  generation: 2,
  revision: 4,
  kind: 'relocate',
  operation: { status: 'completed' },
  disposition: { status: 'restart-selected-root', root: '/moved/foks-rs' },
};

test('restart-required restarts the app before the shell mounts', async () => {
  let restarts = 0;
  const { rendered } = await startupOverlay(restartRequired, {
    restartApp: async () => {
      restarts++;
    },
  });
  assert.match(
    document.body.textContent ?? '',
    /Please restart FOKS to open \/moved\/foks-rs\./,
  );
  const restart = await rendered.findByRole('button', { name: 'Restart FOKS' });
  await ui.act(async () => {
    restart.click();
    await Promise.resolve();
  });
  assert.equal(restarts, 1);
  rendered.unmount();
});

test('restart-required restarts the app from the mounted shell', async () => {
  let restarts = 0;
  const { rendered } = await shellOverlay(restartRequired, {
    restartApp: async () => {
      restarts++;
    },
  });
  assert.match(
    document.querySelector('.stopwrap')?.textContent ?? '',
    /Please restart FOKS to open \/moved\/foks-rs\./,
  );
  const restart = await rendered.findByRole('button', { name: 'Restart FOKS' });
  await ui.act(async () => {
    restart.click();
    await Promise.resolve();
  });
  assert.equal(restarts, 1);
  rendered.unmount();
});

test('recovery-required displays copyable commands and a quit button', async () => {
  const copied: string[] = [];
  let quits = 0;
  const { rendered } = await startupOverlay(
    {
      state: 'complete',
      generation: 3,
      revision: 5,
      kind: 'relocate',
      operation: { status: 'cancelled' },
      disposition: { status: 'recovery-required', root: '/old/foks-rs' },
    },
    {
      copyText: async (text: string) => {
        copied.push(text);
        return { ok: true };
      },
      quitApp: async () => {
        quits++;
      },
    },
  );
  const commands = [...document.querySelectorAll('.copybox code')].map(
    (node) => node.textContent,
  );
  assert.deepEqual(commands, [
    'foks-rs --state-dir /old/foks-rs state status',
    'foks-rs --state-dir /old/foks-rs state recover',
  ]);
  const copy = rendered.getAllByRole('button', { name: 'Copy' });
  assert.equal(copy.length, 2);
  await ui.act(async () => {
    copy[1].click();
    await Promise.resolve();
  });
  assert.deepEqual(copied, ['foks-rs --state-dir /old/foks-rs state recover']);
  const quit = await rendered.findByRole('button', { name: 'Quit FOKS' });
  await ui.act(async () => {
    quit.click();
    await Promise.resolve();
  });
  assert.equal(quits, 1);
  rendered.unmount();
});

test('restoration-failed keeps its retry and adds a quit action', async () => {
  let retries = 0;
  let quits = 0;
  const { rendered } = await startupOverlay(
    {
      state: 'complete',
      generation: 4,
      revision: 6,
      kind: 'verify',
      operation: { status: 'completed' },
      disposition: {
        status: 'restoration-failed',
        root: '/isolated/client',
        error: {
          code: 'agent-stop-pending',
          message: 'The prior agent is still exiting.',
          retryable: true,
          ambiguous: false,
          fatal: false,
        },
      },
    },
    {
      retryAgentConnection: async () => {
        retries++;
        return { state: 'ready' };
      },
      quitApp: async () => {
        quits++;
      },
    },
  );
  assert.match(
    document.body.textContent ?? '',
    /The prior agent is still exiting\./,
  );
  const quit = await rendered.findByRole('button', { name: 'Quit FOKS' });
  await ui.act(async () => {
    quit.click();
    await Promise.resolve();
  });
  assert.equal(quits, 1);
  const retry = await rendered.findByRole('button', {
    name: 'Restart service',
  });
  await ui.act(async () => {
    retry.click();
    await Promise.resolve();
  });
  await ui.waitFor(() => assert.equal(retries, 1));
  rendered.unmount();
});

test('a conceal does not reopen the sheet a scene opened with the page', async () => {
  const { App, FIXTURE, mockBridge } = await modules();
  const listeners = new Set<(value: MaintenanceSnapshot) => void>();
  let current: MaintenanceSnapshot = {
    state: 'idle',
    generation: 0,
    revision: 0,
  };
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    clientStateMaintenanceStatus: async () => current,
    onMaintenanceStatus: async (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
  // `?state=settings-phrase` opens Devices with the one-time paper-key sheet.
  window.history.replaceState(null, '', '/?state=settings-phrase');
  const rendered = ui.render(createElement(App, { bridge }));
  await ui.waitFor(() => {
    assert.match(document.body.textContent ?? '', /Save paper key/);
  });

  // The conceal remounts the tabs to take the phrase off the screen. The scene
  // was entered once: re-reading it here would put the phrase straight back.
  current = {
    state: 'complete',
    generation: 6,
    revision: 8,
    kind: 'verify',
    operation: { status: 'completed' },
    disposition: { status: 'continue-current-root' },
  };
  await ui.act(async () => {
    for (const listener of listeners) listener(current);
    await Promise.resolve();
  });
  await ui.waitFor(() => {
    assert.doesNotMatch(document.body.textContent ?? '', /Save paper key/);
  });
  // The page itself is back, so the phrase is gone rather than the whole tab.
  assert.match(document.body.textContent ?? '', /Paper keys/);
  rendered.unmount();
});

test('active maintenance overlay renders no action buttons', async () => {
  const { rendered } = await shellOverlay({
    state: 'active',
    generation: 5,
    revision: 7,
    kind: 'export',
    phase: 'running',
  });
  const dialog = document.querySelector('.stopwrap');
  assert.ok(dialog);
  assert.match(dialog.textContent ?? '', /export · running/i);
  assert.equal(dialog.querySelectorAll('button').length, 0);
  rendered.unmount();
});
