/**
 * Verifies the launch sequence: one readiness probe, an overlapped appInfo,
 * a loading screen held until catalog and device reads settle, and no team discovery
 * on the launch path.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import type { Bridge, CatalogDto } from '../src/bridge';
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
  const { FIRST_PAINT_DEADLINE_MS } = (await vite.ssrLoadModule(
    '/src/app/app-bootstrap.ts',
  )) as typeof import('../src/app/app-bootstrap');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  return { App, FIRST_PAINT_DEADLINE_MS, FIXTURE, mockBridge };
}

/** A catalog read the test drives: partials on demand, completion on demand. */
function drivenCatalog(): {
  read: Bridge['listCatalog'];
  emit: (partial: CatalogDto) => Promise<void>;
  finish: (response: CatalogDto) => void;
  started: Promise<void>;
  calls: () => number;
} {
  let publish: ((partial: CatalogDto) => void) | undefined;
  let settle!: (response: CatalogDto) => void;
  let entered!: () => void;
  const response = new Promise<CatalogDto>((resolve) => {
    settle = resolve;
  });
  const started = new Promise<void>((resolve) => {
    entered = resolve;
  });
  let calls = 0;
  return {
    calls: () => calls,
    started,
    read: async (onPartial) => {
      calls++;
      publish = onPartial;
      entered();
      return response;
    },
    emit: async (partial) => {
      await started;
      await ui.act(async () => {
        publish?.(partial);
        await Promise.resolve();
      });
    },
    finish: settle,
  };
}

/** The locally known facts the native bridge attaches to every response. */
async function metadataOf(
  bridge: Bridge,
  profiles: readonly string[],
): Promise<CatalogDto['localMetadata']> {
  return {
    accounts: await bridge.listAccounts(),
    profiles: await Promise.all(
      profiles.map(async (profile) => ({
        profile,
        label: null,
        configuredProbe: profile,
        status: await bridge.describeServerStatus(profile),
        error: null,
      })),
    ),
  };
}

const mounted = (): boolean =>
  document.querySelector('.booting') === null &&
  document.querySelector('.app') !== null;

test('launch probes the agent once and overlaps appInfo with the catalog read', async () => {
  const { App, FIXTURE, mockBridge } = await modules();
  const base = mockBridge(FIXTURE);
  const order: string[] = [];
  let statusCalls = 0;
  let statusAtRead = -1;
  let entered!: () => void;
  const readStarted = new Promise<void>((resolve) => {
    entered = resolve;
  });
  const bridge: Bridge = {
    ...base,
    native: true,
    agentStatus: async () => {
      statusCalls++;
      return { state: 'ready' };
    },
    appInfo: async () => {
      order.push('appInfo');
      // Answer only once the catalog read is under way. A launch that awaited
      // appInfo before starting the read would never start it.
      await readStarted;
      order.push('appInfo:done');
      return base.appInfo();
    },
    listCatalog: async (onPartial, fresh) => {
      order.push('listCatalog');
      statusAtRead = statusCalls;
      entered();
      return base.listCatalog(onPartial, fresh);
    },
  };
  const rendered = ui.render(createElement(App, { bridge }));
  try {
    await ui.waitFor(() => assert.ok(mounted()), { timeout: 4_000 });
    assert.deepEqual(order.slice(0, 3), [
      'appInfo',
      'listCatalog',
      'appInfo:done',
    ]);
    assert.equal(statusAtRead, 1, 'the read reuses the readiness probe');
  } finally {
    rendered.unmount();
  }
});

test('the shell does not mount on a skeleton partial and reports its progress', async () => {
  const { App, FIXTURE, mockBridge } = await modules();
  const base = mockBridge(FIXTURE);
  const full = await base.listCatalog();
  const localMetadata = await metadataOf(base, full.profiles);
  const skeleton: CatalogDto = {
    ...full,
    inventory: [],
    items: [],
    localMetadata,
  };
  const driven = drivenCatalog();
  const bridge: Bridge = { ...base, native: true, listCatalog: driven.read };
  const rendered = ui.render(
    createElement(App, { bridge, firstPaintDeadlineMs: 30_000 }),
  );
  try {
    // The read is what the window waits on from the moment it starts, before
    // any profile has reported a count.
    await driven.started;
    await ui.waitFor(() =>
      assert.match(
        document.body.textContent ?? '',
        /Connecting to your vaults/,
      ),
    );
    assert.equal(document.querySelector('.booting .prog'), null);
    await driven.emit(skeleton);
    await ui.waitFor(() =>
      assert.match(
        document.body.textContent ?? '',
        new RegExp(`0 of ${full.profiles.length} profiles ready`),
      ),
    );
    assert.equal(mounted(), false);
    const region = document.querySelector('.booting');
    assert.ok(region);
    assert.equal(region.getAttribute('role'), 'status');
    await driven.emit({ ...skeleton, inventory: [full.inventory[0]] });
    await ui.waitFor(() =>
      assert.match(
        document.body.textContent ?? '',
        new RegExp(`1 of ${full.profiles.length} profiles ready`),
      ),
    );
    // The live region is re-read, not replaced, so each partial does not
    // announce itself as a new region.
    assert.equal(document.querySelector('.booting'), region);
    await ui.act(async () => {
      driven.finish(full);
      await Promise.resolve();
    });
    await ui.waitFor(() => assert.ok(mounted()));
  } finally {
    driven.finish(full);
    rendered.unmount();
  }
});

test('a partial whose stores are still loading holds the paint and says so', async () => {
  const { App, FIXTURE, mockBridge } = await modules();
  const base = mockBridge(FIXTURE);
  const full = await base.listCatalog();
  // Every profile has listed its stores, but none of those stores has been
  // walked yet, so the shell would draw nothing but "loading" rows.
  const listed: CatalogDto = {
    ...full,
    items: [],
    localMetadata: await metadataOf(base, full.profiles),
  };
  const driven = drivenCatalog();
  const bridge: Bridge = { ...base, native: true, listCatalog: driven.read };
  const rendered = ui.render(
    createElement(App, { bridge, firstPaintDeadlineMs: 30_000 }),
  );
  try {
    await driven.emit(listed);
    await ui.waitFor(() =>
      assert.match(document.body.textContent ?? '', /Loading items…/),
    );
    assert.equal(mounted(), false);
  } finally {
    driven.finish(full);
    rendered.unmount();
  }
});

test('ready catalog partials preload devices but startup waits for both reads', async () => {
  const { App, FIXTURE, mockBridge } = await modules();
  const base = mockBridge(FIXTURE);
  const full = await base.listCatalog();
  const ready: CatalogDto = {
    ...full,
    fullItemReads: full.profiles,
    localMetadata: await metadataOf(base, full.profiles),
  };
  const driven = drivenCatalog();
  let releaseDevices!: () => void;
  const devices = new Promise<void>((resolve) => {
    releaseDevices = resolve;
  });
  let deviceReads = 0;
  const bridge: Bridge = {
    ...base,
    native: true,
    listCatalog: driven.read,
    listAccountDevices: async (store) => {
      deviceReads++;
      await devices;
      return base.listAccountDevices(store);
    },
  };
  const rendered = ui.render(
    createElement(App, { bridge, firstPaintDeadlineMs: 30_000 }),
  );
  try {
    await driven.emit(ready);
    await ui.waitFor(() => assert.ok(deviceReads > 0));
    assert.equal(mounted(), false);
    await ui.act(async () => driven.finish(ready));
    assert.equal(mounted(), false);
    await ui.act(async () => releaseDevices());
    await ui.waitFor(() =>
      assert.ok(document.querySelector('.side.rail .who .t')),
    );
    assert.ok(mounted());
  } finally {
    releaseDevices();
    driven.finish(full);
    rendered.unmount();
  }
});

test('the first-paint deadline starts device reads without bypassing catalog completion', async () => {
  const { App, FIRST_PAINT_DEADLINE_MS, FIXTURE, mockBridge } = await modules();
  // The deadline begins preloading; it no longer releases startup itself.
  assert.equal(FIRST_PAINT_DEADLINE_MS, 2_500);
  const base = mockBridge(FIXTURE);
  const full = await base.listCatalog();
  const skeleton: CatalogDto = {
    ...full,
    inventory: [],
    items: [],
    localMetadata: await metadataOf(base, full.profiles),
  };
  const driven = drivenCatalog();
  let deviceReads = 0;
  const bridge: Bridge = {
    ...base,
    native: true,
    listCatalog: driven.read,
    listAccountDevices: async (store) => {
      deviceReads++;
      return base.listAccountDevices(store);
    },
  };
  const rendered = ui.render(
    createElement(App, { bridge, firstPaintDeadlineMs: 40 }),
  );
  try {
    await driven.emit(skeleton);
    assert.equal(mounted(), false);
    await ui.waitFor(() => assert.ok(deviceReads > 0));
    assert.equal(mounted(), false);
    await ui.act(async () => driven.finish(full));
    await ui.waitFor(() => assert.ok(mounted()), { timeout: 4_000 });
  } finally {
    driven.finish(full);
    rendered.unmount();
  }
});

test('an empty catalog partial waits for completion before choosing onboarding', async () => {
  const { App, FIXTURE, mockBridge } = await modules();
  const base = mockBridge(FIXTURE);
  const full = await base.listCatalog();
  const nothing: CatalogDto = {
    ...full,
    profiles: [],
    stores: [],
    knownStores: [],
    inventory: [],
    items: [],
    failures: [],
    blockedProfiles: [],
    localMetadata: { accounts: [], profiles: [] },
  };
  const driven = drivenCatalog();
  const bridge: Bridge = { ...base, native: true, listCatalog: driven.read };
  const rendered = ui.render(
    createElement(App, { bridge, firstPaintDeadlineMs: 30_000 }),
  );
  try {
    await driven.emit(nothing);
    assert.equal(mounted(), false);
    await ui.act(async () => driven.finish(nothing));
    await ui.waitFor(
      () => {
        assert.equal(document.querySelector('.booting'), null);
        assert.ok(
          ui.screen.getByRole('heading', { name: 'How are you joining?' }),
        );
      },
      { timeout: 4_000 },
    );
  } finally {
    driven.finish(full);
    rendered.unmount();
  }
});

test('the lock card wins over the loading screen and reads no catalog', async () => {
  const { App, FIXTURE, mockBridge } = await modules();
  const base = mockBridge(FIXTURE);
  const driven = drivenCatalog();
  const bridge: Bridge = {
    ...base,
    native: true,
    appLockState: async () => ({
      locked: true,
      available: true,
      mechanism: 'password',
    }),
    listCatalog: driven.read,
  };
  const rendered = ui.render(createElement(App, { bridge }));
  try {
    await ui.waitFor(() =>
      assert.match(document.body.textContent ?? '', /FOKS is locked/),
    );
    assert.doesNotMatch(
      document.body.textContent ?? '',
      /Connecting to your vaults/,
    );
    assert.equal(driven.calls(), 0);
  } finally {
    rendered.unmount();
  }
});

test('launch discovers no teams and reads the whole catalog once', async () => {
  const { App, FIXTURE, mockBridge } = await modules();
  const base = mockBridge(FIXTURE);
  const full = await base.listCatalog();
  // An account with no team store is exactly what launch used to discover for.
  const unbound: CatalogDto = {
    ...full,
    stores: full.stores.filter((store) => store.kind !== 'team'),
    knownStores: full.knownStores.filter((store) => store.kind !== 'team'),
    items: [],
    fullItemReads: full.profiles,
    localMetadata: await metadataOf(base, full.profiles),
  };
  let reads = 0;
  let discoveries = 0;
  const bridge: Bridge = {
    ...base,
    native: true,
    listCatalog: async () => {
      reads++;
      return unbound;
    },
    discoverGroups: async (profile, alias) => {
      discoveries++;
      return base.discoverGroups(profile, alias);
    },
  };
  const rendered = ui.render(createElement(App, { bridge }));
  try {
    await ui.waitFor(() => assert.ok(mounted()), { timeout: 4_000 });
    assert.equal(discoveries, 0);
    assert.equal(reads, 1);
  } finally {
    rendered.unmount();
  }
});
