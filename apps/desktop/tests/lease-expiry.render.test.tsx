import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import type { LeaseExpiryClock } from '../src/scheduling/lease-expiry';
import type { Bridge, MaintenanceSnapshot } from '../src/bridge';
import { installDom } from './lib/dom-harness';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
});

class Clock implements LeaseExpiryClock {
  seconds = 1_000;
  private sequence = 0;
  private timers = new Map<number, { at: number; callback: () => void }>();
  now = (): number => this.seconds;
  later = (callback: () => void, delayMs: number): unknown => {
    const id = ++this.sequence;
    this.timers.set(id, { at: this.seconds * 1_000 + delayMs, callback });
    return id;
  };
  cancel = (timer: unknown): void => {
    if (typeof timer === 'number') this.timers.delete(timer);
  };
  advance(seconds: number): void {
    this.seconds += seconds;
    for (;;) {
      const due = [...this.timers.entries()].find(
        ([, timer]) => timer.at <= this.seconds * 1_000,
      );
      if (!due) return;
      this.timers.delete(due[0]);
      due[1].callback();
    }
  }
}

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

test('an expiring open vault conceals details while a healthy neighbor stays usable', async () => {
  const { App } = (await vite.ssrLoadModule(
    '/src/app-root.tsx',
  )) as typeof import('../src/app-root');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const { LocationStore, INITIAL_STATE } = (await vite.ssrLoadModule(
    '/src/location.ts',
  )) as typeof import('../src/location');
  const item = FIXTURE.items.find(
    (entry) => entry.store === 'team:eng' && entry.kind === 'Secret',
  );
  assert.ok(item);
  const start = Date.now() / 1_000;
  const expiresAt = Math.ceil(start) + 10;
  const agentSnapshot = {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server) =>
      server.id === 'acme'
        ? {
            ...server,
            compatibility: { status: 'required' as const, expiresAt },
          }
        : server,
    ),
    observedExpiredLeases: [],
  };
  const locations = new LocationStore({
    ...INITIAL_STATE,
    location: { kind: 'store', ref: 'team:eng' },
    selection: { store: item.store, path: item.path },
    details: true,
  });
  const clock = new Clock();
  clock.seconds = start;
  const rendered = ui.render(
    createElement(App, {
      snapshot: agentSnapshot,
      bridge: mockBridge(agentSnapshot),
      store: locations,
      leaseClock: clock,
    }),
  );
  await ui.waitFor(() => assert.ok(rendered.getByLabelText('Details')));
  const show = rendered.queryByRole('button', { name: 'Show' });
  if (show) ui.fireEvent.click(show);
  await ui.waitFor(() =>
    assert.ok(rendered.getByText('foks_team_token_7f31ac09')),
  );

  await ui.act(async () => clock.advance(expiresAt - start));
  await ui.waitFor(() => {
    assert.equal(rendered.queryByLabelText('Details'), null);
    assert.equal(rendered.queryByText('foks_team_token_7f31ac09'), null);
    assert.ok(rendered.getAllByText('Check-in expired').length > 0);
  });
  // The rail does not list stores; the healthy neighbour is a row on Files.
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Files home' }));
  const personal = await ui.waitFor(() => {
    const row = (rendered.getAllByRole('button') as HTMLButtonElement[]).find(
      (button) => button.textContent?.includes('Personal'),
    );
    assert.ok(row);
    return row;
  });
  assert.equal(personal.disabled, false);
});

test('expiry on one profile preserves a healthy neighboring editor draft', async () => {
  const { App } = (await vite.ssrLoadModule(
    '/src/app-root.tsx',
  )) as typeof import('../src/app-root');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const { LocationStore, INITIAL_STATE } = (await vite.ssrLoadModule(
    '/src/location.ts',
  )) as typeof import('../src/location');
  const item = FIXTURE.items.find(
    (entry) =>
      entry.store === 'acct:personal' && entry.path === '/logins/github.com',
  );
  assert.ok(item);
  const start = Date.now() / 1_000;
  const expiresAt = Math.ceil(start) + 10;
  const agentSnapshot = {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server) =>
      server.id === 'acme'
        ? {
            ...server,
            compatibility: { status: 'required' as const, expiresAt },
          }
        : server,
    ),
    observedExpiredLeases: [],
  };
  const locations = new LocationStore({
    ...INITIAL_STATE,
    location: { kind: 'store', ref: 'acct:personal' },
    selection: { store: item.store, path: item.path },
    details: true,
  });
  const clock = new Clock();
  clock.seconds = start;
  const rendered = ui.render(
    createElement(App, {
      snapshot: agentSnapshot,
      bridge: mockBridge(agentSnapshot),
      store: locations,
      leaseClock: clock,
    }),
  );
  ui.fireEvent.click(await rendered.findByRole('button', { name: 'Edit' }));
  const username = await rendered.findByLabelText('User name');
  ui.fireEvent.change(username, { target: { value: 'preserved-draft' } });

  await ui.act(async () => clock.advance(expiresAt - start));
  assert.equal(
    (rendered.getByLabelText('User name') as HTMLInputElement).value,
    'preserved-draft',
  );
  assert.ok(rendered.getByLabelText('Details'));
});

test('foreground retries authenticated reconciliation after an expiry refresh fails', async () => {
  const { App } = (await vite.ssrLoadModule(
    '/src/app-root.tsx',
  )) as typeof import('../src/app-root');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const start = Date.now() / 1_000;
  const expiresAt = Math.ceil(start) + 10;
  const agentSnapshot = {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server) =>
      server.id === 'acme'
        ? {
            ...server,
            compatibility: { status: 'required' as const, expiresAt },
          }
        : server,
    ),
    observedExpiredLeases: [],
  };
  const base = mockBridge(agentSnapshot);
  let catalogs = 0;
  const bridge = {
    ...base,
    listCatalog: async () => {
      catalogs++;
      if (catalogs === 1)
        throw {
          code: 'offline',
          message: 'Temporarily offline.',
          retryable: true,
          ambiguous: false,
          fatal: false,
        };
      return base.listCatalog();
    },
  };
  const clock = new Clock();
  clock.seconds = start;
  ui.render(
    createElement(App, { snapshot: agentSnapshot, bridge, leaseClock: clock }),
  );
  await ui.act(async () => clock.advance(expiresAt - start));
  await ui.waitFor(() => assert.equal(catalogs, 1));
  Object.defineProperty(document, 'hidden', {
    configurable: true,
    value: false,
  });
  ui.fireEvent(document, new Event('visibilitychange'));
  await ui.waitFor(() => assert.equal(catalogs, 2));
});

test('an authoritative zero-profile catalog enters fresh-install onboarding', async () => {
  const { App } = (await vite.ssrLoadModule(
    '/src/app-root.tsx',
  )) as typeof import('../src/app-root');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  window.history.replaceState(null, '', '/');
  const empty = {
    ...FIXTURE,
    servers: [],
    accounts: [],
    stores: [],
    storeInventory: [],
    profileInventory: [],
    catalogProfiles: [],
    profileInventoryStatus: 'complete' as const,
    items: [],
    parties: [],
    federation: [],
    groupDetailFailures: [],
    notifications: [],
    observedExpiredLeases: [],
    plaintext: {},
  };
  ui.render(createElement(App, { bridge: mockBridge(empty) }));
  await ui.waitFor(() =>
    assert.ok(ui.screen.getByText('How are you joining?')),
  );
});

test('expiry during native maintenance waits for lifecycle restoration to refresh', async () => {
  const { App } = (await vite.ssrLoadModule(
    '/src/app-root.tsx',
  )) as typeof import('../src/app-root');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const start = Date.now() / 1_000;
  const expiresAt = Math.ceil(start) + 10;
  const agentSnapshot = {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server) =>
      server.id === 'acme'
        ? {
            ...server,
            compatibility: { status: 'required' as const, expiresAt },
          }
        : server,
    ),
    observedExpiredLeases: [],
  };
  let snapshot: MaintenanceSnapshot = {
    state: 'idle',
    generation: 0,
    revision: 0,
  };
  const listeners = new Set<(value: MaintenanceSnapshot) => void>();
  let catalogs = 0;
  const base = mockBridge(agentSnapshot);
  const bridge: Bridge = {
    ...base,
    native: true,
    clientStateMaintenanceStatus: async () => snapshot,
    onMaintenanceStatus: async (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    listCatalog: async () => {
      catalogs++;
      return base.listCatalog();
    },
  };
  const clock = new Clock();
  clock.seconds = start;
  ui.render(
    createElement(App, { snapshot: agentSnapshot, bridge, leaseClock: clock }),
  );
  await ui.waitFor(() => assert.equal(listeners.size, 1));
  snapshot = {
    state: 'active',
    generation: 1,
    revision: 1,
    kind: 'verify',
    phase: 'running',
  };
  await ui.act(async () => {
    for (const listener of listeners) listener(snapshot);
  });
  await ui.act(async () => clock.advance(expiresAt - start));
  assert.equal(catalogs, 0);

  snapshot = {
    state: 'complete',
    generation: 1,
    revision: 2,
    kind: 'verify',
    operation: { status: 'completed' },
    disposition: { status: 'continue-current-root' },
  };
  await ui.act(async () => {
    for (const listener of listeners) listener(snapshot);
  });
  await ui.waitFor(() => assert.ok(catalogs >= 1));
});

test('a forced expiry refresh waits for the in-flight foreground load', async () => {
  const { App } = (await vite.ssrLoadModule(
    '/src/app-root.tsx',
  )) as typeof import('../src/app-root');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const start = Date.now() / 1_000;
  const expiresAt = Math.ceil(start) + 10;
  const agentSnapshot = {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server) =>
      server.id === 'acme'
        ? {
            ...server,
            compatibility: { status: 'required' as const, expiresAt },
          }
        : server,
    ),
    observedExpiredLeases: [],
  };
  const base = mockBridge(agentSnapshot);
  // Every catalog load waits until released, so a second load can only start
  // once the test lets the first one finish.
  const releases: Array<() => void> = [];
  let catalogs = 0;
  const bridge = {
    ...base,
    listCatalog: async () => {
      catalogs++;
      await new Promise<void>((resolve) => releases.push(resolve));
      return base.listCatalog();
    },
  };
  const clock = new Clock();
  clock.seconds = start;
  const rendered = ui.render(
    createElement(App, { snapshot: agentSnapshot, bridge, leaseClock: clock }),
  );
  await rendered.findByRole('button', { name: /Refresh/ });
  Object.defineProperty(document, 'hidden', {
    configurable: true,
    value: false,
  });
  ui.fireEvent(document, new Event('visibilitychange'));
  await ui.waitFor(() => assert.equal(catalogs, 1));
  await ui.act(async () => clock.advance(expiresAt - start));
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(catalogs, 1, 'the forced load must not overlap the live one');
  await ui.act(async () => releases.shift()?.());
  await ui.waitFor(() => assert.equal(catalogs, 2));
  await ui.act(async () => releases.shift()?.());
});
