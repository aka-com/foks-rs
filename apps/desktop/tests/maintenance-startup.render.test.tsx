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

test.after(async () => {
  await vite.close();
  dom.window.close();
});

test('startup mounts local catalog shells before a blocked profile and hands off maintenance once', async () => {
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
  const full = await base.listCatalog();
  const local = {
    ...full,
    stores: [],
    inventory: [],
    items: [],
    failures: [],
    blockedProfiles: [],
  };
  let finish!: (value: typeof full) => void;
  const pending = new Promise<typeof full>((resolve) => {
    finish = resolve;
  });
  const listeners = new Set<(value: MaintenanceSnapshot) => void>();
  let maintenance: MaintenanceSnapshot = {
    state: 'idle',
    generation: 0,
    revision: 0,
  };
  const bridge: Bridge = {
    ...base,
    native: true,
    listCatalog: async (onPartial) => {
      onPartial?.(local);
      return pending;
    },
    onMaintenanceStatus: async (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    clientStateMaintenanceStatus: async () => maintenance,
  };
  const rendered = ui.render(createElement(App, { bridge }));
  try {
    await ui.waitFor(() => {
      assert.doesNotMatch(
        document.body.textContent ?? '',
        /Starting the FOKS agent/,
      );
      assert.ok(document.querySelector('.app'));
    });
    assert.doesNotMatch(document.body.textContent ?? '', /Welcome to FOKS/);
    await ui.waitFor(() => assert.equal(listeners.size, 1));
    await ui.act(async () => {
      maintenance = {
        state: 'active',
        generation: 9,
        revision: 1,
        kind: 'verify',
        phase: 'running',
      };
      for (const listener of listeners) listener(maintenance);
      finish(full);
    });
    await ui.waitFor(() =>
      assert.match(
        document.body.textContent ?? '',
        /paused while background maintenance/,
      ),
    );
    assert.equal(listeners.size, 1);
  } finally {
    finish(full);
    rendered.unmount();
  }
});

test('healthy startup progress renders and an older boot cannot overwrite a shell refresh', async () => {
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
  const full = await base.listCatalog();
  const item = full.items[0];
  const partial = {
    ...full,
    profiles: [...full.profiles, 'unfinished'],
    fullItemReads: full.profiles,
    items: [{ ...item, path: '/boot-partial-marker' }],
    localMetadata: {
      accounts: await base.listAccounts(),
      profiles: await Promise.all(
        full.profiles.map(async (profile) => ({
          profile,
          label: null,
          configuredProbe: profile,
          status: await base.describeServerStatus(profile),
          error: null,
        })),
      ),
    },
  };
  let finish!: (value: typeof full) => void;
  const pending = new Promise<typeof full>((resolve) => {
    finish = resolve;
  });
  let calls = 0;
  const bridge: Bridge = {
    ...base,
    native: true,
    listCatalog: async (onPartial) => {
      if (++calls === 1) {
        onPartial?.(partial);
        return pending;
      }
      return { ...full, items: [{ ...item, path: '/new-catalog-marker' }] };
    },
    listProfileCatalog: async (profile) => {
      const response = await base.listProfileCatalog(profile);
      return {
        ...response,
        items: response.items.map((entry) =>
          entry.store === item.store && entry.path === item.path
            ? { ...entry, path: '/new-catalog-marker' }
            : entry,
        ),
      };
    },
  };
  const rendered = ui.render(createElement(App, { bridge }));
  try {
    await ui.waitFor(() =>
      assert.match(document.body.textContent ?? '', /boot-partial-marker/),
    );
    await ui.act(async () => {
      window.dispatchEvent(new dom.window.Event('focus'));
    });
    await ui.waitFor(() =>
      assert.match(document.body.textContent ?? '', /new-catalog-marker/),
    );
    await ui.act(async () => {
      finish(full);
      await Promise.resolve();
    });
    assert.match(document.body.textContent ?? '', /new-catalog-marker/);
    assert.doesNotMatch(document.body.textContent ?? '', /boot-partial-marker/);
  } finally {
    finish(full);
    rendered.unmount();
    Reflect.deleteProperty(document, 'hidden');
  }
});

test('startup paused by maintenance resumes from the terminal native event', async () => {
  const { App } = (await vite.ssrLoadModule(
    '/src/app-root.tsx',
  )) as typeof import('../src/app-root');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  let snapshot: MaintenanceSnapshot = {
    state: 'active',
    generation: 3,
    revision: 7,
    kind: 'verify',
    phase: 'running',
  };
  const maintenanceListeners = new Set<(value: MaintenanceSnapshot) => void>();
  let statusCalls = 0;
  let catalogCalls = 0;
  const base = mockBridge(FIXTURE);
  const bridge: Bridge = {
    ...base,
    native: true,
    clientStateMaintenanceStatus: async () => snapshot,
    onMaintenanceStatus: async (listener) => {
      maintenanceListeners.add(listener);
      return () => maintenanceListeners.delete(listener);
    },
    agentStatus: async () => {
      statusCalls++;
      return { state: 'ready' };
    },
    listCatalog: async () => {
      catalogCalls++;
      return base.listCatalog();
    },
  };

  const rendered = ui.render(createElement(App, { bridge }));
  // The frame is drawn from the first paint; maintenance holds the content
  // area at the starting screen, which names no internal operation.
  await ui.waitFor(() => {
    assert.match(document.body.textContent ?? '', /Starting the FOKS agent…/);
  });
  assert.equal(document.querySelector('.side.rail .who .t'), null);
  assert.equal(statusCalls, 0);
  assert.equal(catalogCalls, 0);

  snapshot = {
    state: 'complete',
    generation: 3,
    revision: 8,
    kind: 'verify',
    operation: { status: 'completed' },
    disposition: { status: 'continue-current-root' },
  };
  assert.equal(maintenanceListeners.size, 1);
  for (const listener of maintenanceListeners) listener(snapshot);
  await ui.waitFor(() => {
    assert.ok(document.querySelector('.app'));
  });
  await ui.waitFor(() => assert.equal(maintenanceListeners.size, 1));
  assert.ok(statusCalls >= 1);
  assert.ok(catalogCalls >= 1);
  rendered.unmount();
});

test('mounted shell ignores duplicate maintenance completion side effects', async () => {
  const { App } = (await vite.ssrLoadModule(
    '/src/app-root.tsx',
  )) as typeof import('../src/app-root');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  let snapshot: MaintenanceSnapshot = {
    state: 'idle',
    generation: 0,
    revision: 0,
  };
  const listeners = new Set<(value: MaintenanceSnapshot) => void>();
  let statusCalls = 0;
  let catalogCalls = 0;
  // Freeze lease time; this test advances maintenance, not lease expiry.
  const leaseClock = {
    now: () => 1_000,
    later: () => 0,
    cancel: () => {},
  };
  let releaseCatalog!: () => void;
  const catalogGate = new Promise<void>((resolve) => {
    releaseCatalog = resolve;
  });
  let catalogEntered!: () => void;
  const catalogStarted = new Promise<void>((resolve) => {
    catalogEntered = resolve;
  });
  const base = mockBridge(FIXTURE);
  const bridge: Bridge = {
    ...base,
    native: true,
    clientStateMaintenanceStatus: async () => snapshot,
    onMaintenanceStatus: async (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    agentStatus: async () => {
      statusCalls++;
      return { state: 'ready' };
    },
    listCatalog: async () => {
      catalogCalls++;
      if (snapshot.state === 'complete' && snapshot.generation === 1) {
        catalogEntered();
        await catalogGate;
      }
      return base.listCatalog();
    },
  };
  const rendered = ui.render(createElement(App, { bridge, leaseClock }));
  // The rail draws before a snapshot loads, so the account header is what says
  // the shell itself has mounted.
  await ui.waitFor(() => {
    assert.ok(document.querySelector('.side.rail .who .t'));
  });
  await ui.waitFor(() => assert.equal(listeners.size, 1));

  snapshot = {
    state: 'active',
    generation: 1,
    revision: 1,
    kind: 'export',
    phase: 'running',
  };
  await ui.act(async () => {
    for (const listener of listeners) listener(snapshot);
  });
  assert.match(
    document.querySelector('.stopveil')?.textContent ?? '',
    /export · running/i,
  );

  snapshot = {
    state: 'complete',
    generation: 1,
    revision: 2,
    kind: 'export',
    operation: { status: 'completed' },
    disposition: { status: 'continue-current-root' },
  };
  await ui.act(async () => {
    for (const listener of listeners) listener(snapshot);
    // Completion establishes readiness before starting a catalog refresh.
    // Wait for the bridge boundary, not a fixed number of microtasks.
    await catalogStarted;
  });
  const callsDuringRefresh = { statusCalls, catalogCalls };
  await ui.act(async () => {
    for (const listener of listeners) listener(snapshot);
  });
  assert.deepEqual(
    { statusCalls, catalogCalls },
    callsDuringRefresh,
    'duplicate completion does not start another refresh while one is pending',
  );
  await ui.act(async () => {
    releaseCatalog();
  });
  await ui.waitFor(() =>
    assert.equal(document.querySelector('.stopveil'), null),
  );
  const callsAfterCompletion = { statusCalls, catalogCalls };
  await ui.act(async () => {
    for (const listener of listeners) listener(snapshot);
  });
  assert.deepEqual(
    { statusCalls, catalogCalls },
    callsAfterCompletion,
    'duplicate terminal event causes no readiness or catalog side effects',
  );
  rendered.unmount();
});

test('startup restoration retry continues through catalog load into the shell', async () => {
  const { App } = (await vite.ssrLoadModule(
    '/src/app-root.tsx',
  )) as typeof import('../src/app-root');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  let snapshot: MaintenanceSnapshot = {
    state: 'complete',
    generation: 4,
    revision: 11,
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
  };
  const listeners = new Set<(value: MaintenanceSnapshot) => void>();
  let statusCalls = 0;
  let catalogCalls = 0;
  const base = mockBridge(FIXTURE);
  const bridge: Bridge = {
    ...base,
    native: true,
    clientStateMaintenanceStatus: async () => snapshot,
    onMaintenanceStatus: async (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    retryAgentConnection: async () => {
      snapshot = {
        state: 'complete',
        generation: 4,
        revision: 12,
        kind: 'verify',
        operation: { status: 'completed' },
        disposition: { status: 'continue-current-root' },
      };
      for (const listener of listeners) listener(snapshot);
      return { state: 'ready' };
    },
    agentStatus: async () => {
      statusCalls++;
      return { state: 'ready' };
    },
    listCatalog: async () => {
      catalogCalls++;
      return base.listCatalog();
    },
  };

  const rendered = ui.render(createElement(App, { bridge }));
  const retry = await rendered.findByRole('button', {
    name: 'Restart service',
  });
  assert.match(
    document.body.textContent ?? '',
    /State maintenance completed\./,
  );
  assert.match(
    document.body.textContent ?? '',
    /The prior agent is still exiting\./,
  );
  assert.equal(listeners.size, 1);
  await ui.act(async () => {
    retry.click();
    await Promise.resolve();
    await Promise.resolve();
  });
  await ui.waitFor(() =>
    assert.ok(document.querySelector('.side.rail .who .t')),
  );
  assert.ok(statusCalls >= 1);
  assert.ok(catalogCalls >= 1);
  assert.equal(listeners.size, 1);
  rendered.unmount();
});

test('maintenance invalidates a pending startup catalog before shell handoff', async () => {
  const { App } = (await vite.ssrLoadModule(
    '/src/app-root.tsx',
  )) as typeof import('../src/app-root');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  let snapshot: MaintenanceSnapshot = {
    state: 'idle',
    generation: 0,
    revision: 0,
  };
  const listeners = new Set<(value: MaintenanceSnapshot) => void>();
  let resolveFirstCatalog: (() => void) | undefined;
  let catalogCalls = 0;
  const firstCatalog = new Promise<void>((resolve) => {
    resolveFirstCatalog = resolve;
  });
  const base = mockBridge(FIXTURE);
  const bridge: Bridge = {
    ...base,
    native: true,
    clientStateMaintenanceStatus: async () => snapshot,
    onMaintenanceStatus: async (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    listCatalog: async () => {
      catalogCalls++;
      if (catalogCalls === 1) await firstCatalog;
      return base.listCatalog();
    },
  };

  const rendered = ui.render(createElement(App, { bridge }));
  await ui.waitFor(() => assert.equal(catalogCalls, 1));
  snapshot = {
    state: 'active',
    generation: 8,
    revision: 20,
    kind: 'relocate',
    phase: 'running',
  };
  await ui.act(async () => {
    for (const listener of listeners) listener(snapshot);
    resolveFirstCatalog?.();
    await Promise.resolve();
  });
  await ui.waitFor(() => {
    assert.match(document.body.textContent ?? '', /Starting the FOKS agent…/);
  });
  assert.equal(document.querySelector('.side.rail .who .t'), null);
  assert.equal(
    catalogCalls,
    1,
    'the invalidated catalog was not handed to VaultShell',
  );

  snapshot = {
    state: 'complete',
    generation: 8,
    revision: 21,
    kind: 'relocate',
    operation: { status: 'cancelled' },
    disposition: { status: 'continue-current-root' },
  };
  await ui.act(async () => {
    for (const listener of listeners) listener(snapshot);
    await Promise.resolve();
  });
  await ui.waitFor(() =>
    assert.ok(document.querySelector('.side.rail .who .t')),
  );
  assert.ok(catalogCalls >= 2);
  assert.equal(listeners.size, 1);
  rendered.unmount();
});
