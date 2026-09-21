import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, useRef, useState } from 'react';
import { installDom } from './lib/dom-harness';
import { useDesktopReconciliation } from '../src/use-desktop-reconciliation';
import {
  profileConnectivityKey,
  profileRefreshKey,
  type DesktopReconciliation,
} from '../src/desktop-reconciliation';
import { CatalogReadGate } from '../src/catalog-read-gate';
import {
  useCatalogRuntime,
  useMutationError,
} from '../src/app/catalog-runtime';
import { AccessLifetime } from '../src/app/access-lifetime';
import { ToastController } from '../kit/toasts';
import { mockBridge } from '../src/mock-bridge';
import { FIXTURE } from '../src/fixture';
import type { Bridge, CatalogDto } from '../src/bridge';
import { serverAvailability, type AgentSnapshot } from '../src/model';
import type { MutationFailureHandler } from '../src/mutation-recovery';
import type { ReconciliationClock } from '../src/scheduling/reconciliation';

installDom({ url: 'http://localhost/', timers: true, act: true });
let ui: typeof import('@testing-library/react');
test.before(async () => {
  ui = await import('@testing-library/react');
});
test.afterEach(() => ui.cleanup());
test('whole-catalog runtime failures do not become failures of every profile and store', async () => {
  const base = mockBridge(FIXTURE);
  let failing = true;
  const bridge: Bridge = {
    ...base,
    listCatalog: async () => {
      if (failing) throw new Error('Catalog projection failed.');
      return base.listCatalog();
    },
  };
  const lifetime = new AccessLifetime();
  const toasts = new ToastController();
  const retireBoot = () => {};
  const currentBootSnapshot = () => true;
  let runtime!: ReturnType<typeof useCatalogRuntime>;
  function CatalogHarness() {
    runtime = useCatalogRuntime({
      lifetime,
      bridge,
      agentSnapshot: FIXTURE,
      retireBoot,
      currentBootSnapshot,
      toasts,
    });
    return null;
  }
  ui.render(createElement(CatalogHarness));
  await ui.act(async () => {
    await assert.rejects(
      runtime.refreshSnapshot(true),
      /Catalog projection failed/,
    );
  });
  const freshness = runtime.latestRef.current.catalogFreshness!;
  assert.ok(
    Object.values(freshness.profiles).every(
      (entry) => !entry.error && !entry.refreshing,
    ),
  );
  assert.ok(
    Object.values(freshness.stores).every(
      (entry) => !entry.error && !entry.refreshing,
    ),
  );
  assert.equal(freshness.attempt?.error?.message, 'Catalog projection failed.');
  assert.strictEqual(
    runtime.latestRef.current.notifications,
    FIXTURE.notifications,
  );
  assert.strictEqual(
    runtime.latestRef.current.storeInventory,
    FIXTURE.storeInventory,
  );
  failing = false;
  await ui.act(async () => {
    await runtime.refreshSnapshot(true);
  });
  assert.equal(
    runtime.latestRef.current.catalogFreshness?.attempt?.error,
    undefined,
  );
  assert.ok(runtime.latestRef.current.catalogFreshness?.attempt?.lastSuccessAt);
});

class Clock implements ReconciliationClock {
  time = 1_000;
  id = 0;
  tasks = new Map<number, { at: number; fn: () => void }>();
  now = () => this.time;
  random = () => 0;
  later = (fn: () => void, delay: number) => {
    const id = ++this.id;
    this.tasks.set(id, { at: this.time + delay, fn });
    return id;
  };
  cancel = (id: unknown) => {
    this.tasks.delete(id as number);
  };
  async advance(ms: number) {
    const end = this.time + ms;
    for (;;) {
      const next = [...this.tasks].sort((a, b) => a[1].at - b[1].at)[0];
      if (!next || next[1].at > end) break;
      this.time = next[1].at;
      this.tasks.delete(next[0]);
      await ui.act(async () => {
        next[1].fn();
        for (let i = 0; i < 100; i++) await Promise.resolve();
      });
    }
    this.time = end;
  }
}
function Harness({
  bridge,
  initial,
  clock,
  enabled = true,
  publish,
  expose,
}: {
  bridge: Bridge;
  initial: AgentSnapshot;
  clock: Clock;
  enabled?: boolean;
  publish(this: void, snapshot: AgentSnapshot): void;
  expose?(this: void, service: DesktopReconciliation): void;
}) {
  const [snapshot, setSnapshot] = useState(initial);
  const latest = useRef(snapshot);
  latest.current = snapshot;
  const [gate] = useState(() => new CatalogReadGate());
  const service = useDesktopReconciliation({
    bridge,
    snapshot,
    enabled,
    gate,
    clock,
    current: () => latest.current,
    publish: (next) => {
      latest.current = next;
      setSnapshot(next);
      publish(next);
    },
    refresh: async () => {},
    metadata: async () => {},
    report: () => {},
    nowSeconds: () => clock.now() / 1_000,
  });
  expose?.(service);
  return createElement(
    'output',
    null,
    snapshot.items.map((item) => `${item.path}:${item.version}`).join(','),
  );
}
async function fixture() {
  Object.defineProperty(document, 'hidden', {
    configurable: true,
    value: false,
  });
  const initial: AgentSnapshot = {
    ...FIXTURE,
    accounts: [],
    servers: FIXTURE.servers.map((server) => ({
      ...server,
      compatibility: { status: 'not-required' },
    })),
  };
  const mock = mockBridge({ ...initial, accounts: FIXTURE.accounts });
  let remote = await mock.listCatalog();
  let reads = 0;
  const bridge: Bridge = {
    ...mock,
    native: true,
    fixtureSnapshot: undefined,
    listStores: async () => remote,
    listProfileCatalog: async (profile): Promise<CatalogDto> => {
      reads++;
      const stores = remote.stores.filter((store) => store.server === profile);
      const ids = new Set(stores.map((store) => store.id));
      return {
        ...remote,
        profiles: [profile],
        stores,
        knownStores: stores,
        inventory: remote.inventory.filter(
          (entry) => entry.profile === profile,
        ),
        items: remote.items.filter((item) => ids.has(item.store)),
        failures: [],
        blockedProfiles: [],
        fullItemReads: [profile],
        storeReads: stores.map((store) => ({
          store: store.id,
          state: 'complete',
        })),
        localMetadata: {
          accounts: (await mock.listAccounts()).filter(
            (account) => account.server === profile,
          ),
          profiles: [
            {
              profile,
              label: null,
              configuredProbe: initial.servers.find(
                (server) => server.id === profile,
              )!.configuredProbe,
              status: await mock.describeServerStatus(profile),
              error: null,
            },
          ],
        },
      };
    },
  };
  return {
    initial,
    bridge,
    reads: () => reads,
    setRemote: (next: CatalogDto) => {
      remote = next;
    },
    remote,
  };
}

test('periodic native profile reads catch remote edits and deletes without manual Refresh', async () => {
  const data = await fixture(),
    clock = new Clock();
  let accepted = data.initial;
  const target = data.remote.items[0];
  assert.ok(target);
  const rendered = ui.render(
    createElement(Harness, {
      ...data,
      clock,
      publish: (next) => {
        accepted = next;
      },
    }),
  );
  data.setRemote({
    ...data.remote,
    items: data.remote.items.map((item) =>
      item === target ? { ...item, version: item.version + 1 } : item,
    ),
  });
  await clock.advance(30_000);
  assert.ok(data.reads() > 0);
  assert.equal(
    accepted.items.find(
      (item) => item.store === target.store && item.path === target.path,
    )?.version,
    target.version + 1,
  );
  assert.equal(
    new Set(accepted.servers.map((server) => server.id)).size,
    accepted.servers.length,
  );
  data.setRemote({
    ...data.remote,
    items: data.remote.items.filter((item) => item !== target),
  });
  await clock.advance(30_000);
  assert.equal(
    accepted.items.some(
      (item) => item.store === target.store && item.path === target.path,
    ),
    false,
  );
  rendered.unmount();
  assert.equal(clock.tasks.size, 0);
});

test('hidden and disabled sessions do not start work; becoming visible reconciles once', async () => {
  const data = await fixture(),
    clock = new Clock();
  const props = { ...data, clock, publish: () => {} };
  const rendered = ui.render(
    createElement(Harness, { ...props, enabled: false }),
  );
  await clock.advance(90_000);
  assert.equal(data.reads(), 0);
  Object.defineProperty(document, 'hidden', {
    configurable: true,
    value: true,
  });
  await ui.act(async () => {
    document.dispatchEvent(new Event('visibilitychange'));
  });
  rendered.rerender(createElement(Harness, props));
  await clock.advance(90_000);
  assert.equal(data.reads(), 0);
  Object.defineProperty(document, 'hidden', {
    configurable: true,
    value: false,
  });
  await ui.act(async () => {
    document.dispatchEvent(new Event('visibilitychange'));
  });
  await clock.advance(0);
  assert.ok(data.reads() > 0);
  const reads = data.reads();
  await clock.advance(0);
  assert.equal(data.reads(), reads);
});

test('a failed store does not replace a healthy sibling’s successful freshness observation', async () => {
  const data = await fixture(),
    clock = new Clock();
  const failed = data.initial.stores.find((store) =>
    data.initial.stores.some(
      (other) => other.server === store.server && other.id !== store.id,
    ),
  )!;
  const healthy = data.initial.stores.find(
    (store) => store.server === failed.server && store.id !== failed.id,
  )!;
  const bridge: Bridge = {
    ...data.bridge,
    listProfileCatalog: async (profile) => {
      const catalog = await data.bridge.listProfileCatalog(profile);
      if (profile !== failed.server) return catalog;
      return {
        ...catalog,
        fullItemReads: [],
        items: catalog.items.filter((item) => item.store !== failed.id),
        storeReads: catalog.storeReads?.map((entry) =>
          entry.store === failed.id
            ? { ...entry, state: 'failed' as const }
            : entry,
        ),
        failures: [
          {
            scope: 'store',
            profile,
            store: failed.id,
            error: {
              code: 'operation-failed',
              message: 'Store unavailable',
              retryable: true,
              fatal: false,
              ambiguous: false,
            },
          },
        ],
      };
    },
  };
  let accepted = data.initial;
  ui.render(
    createElement(Harness, {
      ...data,
      bridge,
      clock,
      publish: (next) => {
        accepted = next;
      },
    }),
  );
  await clock.advance(30_000);
  assert.ok(accepted.catalogFreshness?.profiles[failed.server]?.error);
  assert.ok(accepted.catalogFreshness?.stores[failed.id]?.error);
  assert.equal(accepted.catalogFreshness?.stores[healthy.id]?.error, undefined);
  assert.ok(
    accepted.catalogFreshness?.stores[healthy.id]?.lastSuccessAt !== undefined,
  );
});

test('connectivity recovery refreshes signed facts without replaying setup or blocking healthy profiles', async () => {
  const data = await fixture(),
    clock = new Clock();
  const affected = data.initial.catalogProfiles[0];
  let renewed = false,
    probes = 0,
    accepted = data.initial;
  data.initial.servers = data.initial.servers.map((server) =>
    server.id === affected
      ? {
          ...server,
          compatibility: {
            status: 'required',
            expiresAt: 1,
            capabilities: ['kv'],
          },
        }
      : server,
  );
  const bridge: Bridge = {
    ...data.bridge,
    checkServer: async () =>
      assert.fail('initial trust probing must not be used'),
    reconcileServer: async (profile) => {
      probes++;
      if (profile === affected) renewed = true;
      const status = await data.bridge.describeServerStatus(profile);
      assert.ok(status.host);
      return {
        profile,
        identity: {
          status: 'connected',
          hostId: status.host.hostId,
          configuredProbe: status.configuredProbe,
        },
        compatibility: {
          status: profile === affected ? 'renewed' : 'not-required',
        },
      };
    },
    listProfileCatalog: async (profile) => {
      const result = await data.bridge.listProfileCatalog(profile);
      if (profile !== affected) return result;
      return {
        ...result,
        localMetadata: {
          ...result.localMetadata!,
          profiles: result.localMetadata!.profiles.map((entry) => ({
            ...entry,
            status: {
              ...entry.status!,
              compatibility: {
                status: 'required',
                expiresAt: renewed ? 200 : 1,
                capabilities: ['kv', 'teams', 'chat'],
              },
              leaseRequired: true,
              leaseExpiresAt: renewed ? 200 : 1,
            },
          })),
        },
      };
    },
  };
  ui.render(
    createElement(Harness, {
      ...data,
      bridge,
      clock,
      publish: (next) => {
        accepted = next;
      },
    }),
  );
  await clock.advance(0);
  assert.ok(renewed && probes > 0);
  const server = accepted.servers.find((server) => server.id === affected)!;
  assert.equal(server.connectivity.status, 'observed');
  assert.equal(
    serverAvailability(accepted, server, { nowSeconds: 2 }).available,
    true,
  );
  assert.equal(accepted.agent.state, 'ready');
});

test('catalog refresh failures do not fail connectivity reconciliation', async () => {
  const data = await fixture(),
    clock = new Clock();
  const affected = data.initial.catalogProfiles[0];
  let accepted = data.initial;
  let service!: DesktopReconciliation;
  // The renewed lease is a fact the snapshot does not hold, so the
  // observation goes through the catalog job rather than being published
  // against facts that already say what it says.
  data.initial.servers = data.initial.servers.map((server) =>
    server.id === affected
      ? {
          ...server,
          compatibility: {
            status: 'required',
            expiresAt: 1,
            capabilities: ['kv'],
          },
        }
      : server,
  );
  const bridge: Bridge = {
    ...data.bridge,
    reconcileServer: async (profile) => {
      // The catalog read for this profile fails, so its facts stay as they
      // were; the observation has to name the host those facts hold.
      const server = data.initial.servers.find(
        (server) => server.id === profile,
      );
      const status = await data.bridge.describeServerStatus(profile);
      assert.ok(status.host);
      return {
        profile,
        identity: {
          status: 'connected',
          hostId: server?.host_id ?? status.host.hostId,
          configuredProbe: status.configuredProbe,
        },
        compatibility: {
          status: profile === affected ? 'renewed' : 'not-required',
        },
      };
    },
    listProfileCatalog: async (profile) => {
      if (profile === affected)
        throw {
          code: 'operation-failed',
          message: 'Catalog read failed.',
          retryable: false,
          fatal: true,
          ambiguous: false,
        };
      return data.bridge.listProfileCatalog(profile);
    },
  };
  ui.render(
    createElement(Harness, {
      ...data,
      bridge,
      clock,
      publish: (next) => {
        accepted = next;
      },
      expose: (value) => {
        service = value;
      },
    }),
  );
  await clock.advance(0);
  const connectivity = service.scheduler.snapshot(
    profileConnectivityKey(accepted, affected),
  );
  const catalog = service.scheduler.snapshot(
    profileRefreshKey(accepted, affected),
  );
  // The connectivity job succeeded: its failure would have been the catalog
  // read's, and a fatal one would have parked the connectivity job.
  assert.equal(connectivity?.error, undefined);
  assert.notEqual(connectivity?.paused, true);
  assert.ok(connectivity?.lastSuccessAt);
  // The catalog job carries its own failure, and the fatal one parks it.
  assert.equal(
    (catalog?.error as { message?: string } | undefined)?.message,
    'Catalog read failed.',
  );
  assert.equal(catalog?.paused, true);
  assert.equal(
    accepted.catalogFreshness?.profiles[affected]?.error?.message,
    'Catalog read failed.',
  );
  // The observation was published once the catalog read had settled.
  const server = accepted.servers.find((server) => server.id === affected)!;
  assert.equal(server.connectivity.status, 'observed');
});

test('retiring an active profile refresh clears its refreshing state', async () => {
  const data = await fixture(),
    clock = new Clock();
  const affected = data.initial.catalogProfiles[0];
  let accepted = data.initial;
  let release!: () => void;
  let held = false;
  const bridge: Bridge = {
    ...data.bridge,
    listProfileCatalog: async (profile) => {
      if (profile === affected && !held) {
        held = true;
        await new Promise<void>((done) => {
          release = done;
        });
      }
      return data.bridge.listProfileCatalog(profile);
    },
  };
  const props = {
    ...data,
    bridge,
    clock,
    publish: (next: AgentSnapshot) => {
      accepted = next;
    },
  };
  const rendered = ui.render(createElement(Harness, props));
  await clock.advance(30_000);
  assert.ok(held);
  assert.equal(accepted.catalogFreshness?.profiles[affected]?.refreshing, true);
  // The session is disabled while the read is in flight: the job is retired
  // and the read's answer, when it comes, is not published.
  rendered.rerender(createElement(Harness, { ...props, enabled: false }));
  await ui.act(async () => {
    release();
    for (let i = 0; i < 50; i++) await Promise.resolve();
  });
  assert.equal(
    accepted.catalogFreshness?.profiles[affected]?.refreshing,
    false,
  );
  assert.equal(accepted.catalogFreshness?.profiles[affected]?.error, undefined);
});

/** A connectivity bridge whose observation restates the server's own facts. */
function unchangedConnectivity(
  data: Awaited<ReturnType<typeof fixture>>,
  move: 'none' | 'host' | 'probe' | 'identity' | 'lease',
  moved: string,
): Bridge {
  return {
    ...data.bridge,
    reconcileServer: async (profile) => {
      const status = await data.bridge.describeServerStatus(profile);
      assert.ok(status.host);
      const changed = profile === moved;
      // A projected server carries the host its own facts name, which is what
      // an observation of a server that has not moved reports.
      const server = data.initial.servers.find(
        (candidate) => candidate.id === profile,
      );
      return {
        profile,
        identity:
          changed && move === 'identity'
            ? {
                status: 'failed',
                error: {
                  code: 'io',
                  message: 'The server did not answer.',
                  retryable: true,
                  fatal: false,
                  ambiguous: false,
                },
              }
            : {
                status: 'connected',
                hostId:
                  changed && move === 'host'
                    ? '02ff'
                    : (server?.host_id ?? status.host.hostId),
                configuredProbe:
                  changed && move === 'probe'
                    ? 'moved.example.net'
                    : status.configuredProbe,
              },
        compatibility: {
          status: changed && move === 'lease' ? 'renewed' : 'not-required',
        },
      };
    },
  };
}

test('unchanged connectivity data is published without refreshing the catalog', async () => {
  const data = await fixture(),
    clock = new Clock();
  let accepted = data.initial;
  let rosters = 0;
  const bridge: Bridge = {
    ...unchangedConnectivity(data, 'none', ''),
    listGroupDetails: async (storeId) => {
      rosters++;
      return data.bridge.listGroupDetails(storeId);
    },
  };
  ui.render(
    createElement(Harness, {
      ...data,
      bridge,
      clock,
      publish: (next) => {
        accepted = next;
      },
    }),
  );
  await clock.advance(0);
  assert.equal(data.reads(), 0);
  assert.equal(rosters, 0);
  for (const profile of data.initial.catalogProfiles)
    assert.equal(
      accepted.servers.find((server) => server.id === profile)?.connectivity
        .status,
      'observed',
    );
});

for (const move of ['host', 'probe', 'identity', 'lease'] as const) {
  test(`a moved ${move} still refreshes the profile's catalog`, async () => {
    const data = await fixture(),
      clock = new Clock();
    const moved = data.initial.catalogProfiles[0];
    if (move === 'lease')
      data.initial.servers = data.initial.servers.map((server) =>
        server.id === moved
          ? {
              ...server,
              compatibility: {
                status: 'required',
                expiresAt: 1,
                capabilities: ['kv'],
              },
            }
          : server,
      );
    const reads: string[] = [];
    const base = unchangedConnectivity(data, move, moved);
    const bridge: Bridge = {
      ...base,
      listProfileCatalog: async (profile) => {
        reads.push(profile);
        return base.listProfileCatalog(profile);
      },
    };
    ui.render(
      createElement(Harness, { ...data, bridge, clock, publish: () => {} }),
    );
    await clock.advance(0);
    // Only the profile whose identity changed is refreshed.
    assert.deepEqual([...new Set(reads)], [moved]);
  });
}

test('foreground reconciliation performs one catalog read per unchanged profile', async () => {
  const data = await fixture(),
    clock = new Clock();
  let probes = 0;
  const base = unchangedConnectivity(data, 'none', '');
  const bridge: Bridge = {
    ...base,
    reconcileServer: async (profile) => {
      probes++;
      return base.reconcileServer!(profile);
    },
  };
  ui.render(
    createElement(Harness, { ...data, bridge, clock, publish: () => {} }),
  );
  await clock.advance(60_000);
  const before = { reads: data.reads(), probes };
  assert.ok(before.probes > 0);
  await ui.act(async () => {
    window.dispatchEvent(new Event('focus'));
  });
  await clock.advance(0);
  const walked = data.reads() - before.reads;
  assert.equal(probes - before.probes, data.initial.catalogProfiles.length);
  // One walk per profile, the catalog job's own answer to the wake; the
  // observations that came with it asked for none.
  assert.equal(walked, data.initial.catalogProfiles.length);
});

/**
 * The shell's two runtimes as it wires them: the catalog runtime reads a
 * mutated profile back through the reconciliation service, and reads the
 * whole catalog when that service cannot serve the read.
 */
type Shell = {
  catalog: ReturnType<typeof useCatalogRuntime>;
  reconciliation: DesktopReconciliation;
  mutationError: MutationFailureHandler;
  reported: unknown[];
  toasts: ToastController;
};
function ShellHarness({
  bridge,
  initial,
  clock,
  enabled = true,
  expose,
}: {
  bridge: Bridge;
  initial: AgentSnapshot;
  clock: Clock;
  enabled?: boolean;
  expose(this: void, value: Shell): void;
}) {
  const [lifetime] = useState(() => new AccessLifetime());
  const [toasts] = useState(() => new ToastController());
  const catalog = useCatalogRuntime({
    lifetime,
    bridge,
    agentSnapshot: initial,
    retireBoot: () => {},
    currentBootSnapshot: () => true,
    toasts,
  });
  const reconciliation = useDesktopReconciliation({
    bridge,
    snapshot: catalog.latest,
    enabled,
    gate: catalog.catalogGate,
    clock,
    current: () => catalog.latestRef.current,
    publish: catalog.publishSnapshot,
    refresh: () => catalog.refreshSnapshot(true),
    metadata: async () => {},
    report: () => {},
    nowSeconds: () => clock.now() / 1_000,
  });
  catalog.profileRefresh.current = (profile) =>
    reconciliation.refreshProfile(profile);
  const [reported] = useState<unknown[]>(() => []);
  const mutationError = useMutationError(
    (error) => reported.push(error),
    catalog,
  );
  expose({ catalog, reconciliation, mutationError, reported, toasts });
  return null;
}

async function shell(
  options: { enabled?: boolean; override?: (base: Bridge) => Bridge } = {},
) {
  const data = await fixture();
  const clock = new Clock();
  const reads: string[] = [];
  const whole: number[] = [];
  const rosters: string[] = [];
  const counted: Bridge = {
    ...data.bridge,
    listCatalog: async (publish) => {
      whole.push(clock.now());
      return data.bridge.listCatalog(publish);
    },
    listProfileCatalog: async (profile) => {
      reads.push(profile);
      return data.bridge.listProfileCatalog(profile);
    },
    listGroupDetails: async (storeId) => {
      rosters.push(storeId);
      return data.bridge.listGroupDetails(storeId);
    },
  };
  const bridge = options.override ? options.override(counted) : counted;
  let live!: Shell;
  const toasted: { message: string; tone?: string }[] = [];
  ui.render(
    createElement(ShellHarness, {
      bridge,
      initial: data.initial,
      clock,
      enabled: options.enabled,
      expose: (value) => {
        live = value;
      },
    }),
  );
  live.toasts.connect((request) =>
    toasted.push({ message: request.message, tone: request.tone }),
  );
  return {
    data,
    clock,
    reads,
    whole,
    rosters,
    toasted,
    profile: data.initial.catalogProfiles[0],
    runtime: () => live,
  };
}

test('a mutation refreshes only its affected profile', async () => {
  const harness = await shell();
  let invalidations = 0;
  harness.runtime().catalog.metadataInvalidation.current = () => {
    invalidations++;
  };
  const applied = harness.runtime().catalog.refresh('Saved', harness.profile);
  await harness.clock.advance(0);
  await applied;
  assert.deepEqual(harness.reads, [harness.profile]);
  assert.deepEqual(harness.whole, []);
  for (const store of harness.rosters)
    assert.equal(
      harness.data.initial.stores.find((entry) => entry.id === store)?.server,
      harness.profile,
    );
  // A vault write changes no device metadata, so none of it is discarded
  // and nothing keyed on a forced publication is read again.
  assert.equal(invalidations, 0);
  assert.equal(harness.runtime().catalog.hardwareRefresh, 0);
  assert.deepEqual(harness.toasted, [{ message: 'Saved', tone: undefined }]);
});

test('a retired profile refresh falls back to a full catalog refresh', async () => {
  const harness = await shell();
  const applied = harness.runtime().catalog.refresh('Saved', harness.profile);
  // The scheduler stops between the request and the run, so no read answers
  // it; the write is read back by the only read left rather than silently.
  harness.runtime().reconciliation.scheduler.setVisible(false);
  await harness.clock.advance(0);
  await applied;
  assert.deepEqual(harness.reads, []);
  assert.equal(harness.whole.length, 1);
  assert.deepEqual(harness.toasted, [{ message: 'Saved', tone: undefined }]);
});

test('a failed mutation uses full catalog fallback when its profile refresh is retired', async () => {
  const harness = await shell();
  const item = harness.data.initial.items.find(
    (candidate) =>
      harness.data.initial.stores.find((store) => store.id === candidate.store)
        ?.server === harness.profile,
  );
  assert.ok(item);
  const recovered = harness.runtime().mutationError(
    {
      code: 'operation-failed',
      message: 'The write failed.',
      retryable: true,
      fatal: false,
      ambiguous: false,
    },
    { item, report: false },
  );
  harness.runtime().reconciliation.scheduler.setVisible(false);
  await harness.clock.advance(0);
  await recovered;
  assert.deepEqual(harness.reads, []);
  assert.equal(harness.whole.length, 1);
});

test('a write with no profile, and one the service cannot read, fall back to the whole catalog', async () => {
  for (const enabled of [true, false]) {
    const harness = await shell({ enabled });
    const applied = harness
      .runtime()
      .catalog.refresh('Saved', enabled ? undefined : harness.profile);
    await harness.clock.advance(0);
    await applied;
    assert.equal(harness.whole.length, 1);
    assert.deepEqual(harness.reads, []);
    assert.deepEqual(harness.toasted, [{ message: 'Saved', tone: undefined }]);
    ui.cleanup();
  }
});

test('a failed post-mutation profile refresh shows a warning instead of success', async () => {
  const harness = await shell({
    override: (base) => ({
      ...base,
      listProfileCatalog: async (profile) => {
        await base.listProfileCatalog(profile);
        throw {
          code: 'operation-failed',
          message: 'Vault unavailable.',
          retryable: true,
          fatal: false,
          ambiguous: false,
        };
      },
    }),
  });
  const applied = harness.runtime().catalog.refresh('Saved', harness.profile);
  await harness.clock.advance(0);
  await applied;
  assert.deepEqual(harness.toasted, [
    {
      message:
        'Change completed. Updated data could not be loaded. Use Refresh to reload it.',
      tone: 'warning',
    },
  ]);
});

test('a failed item mutation refreshes the affected profile', async () => {
  const harness = await shell();
  const item = harness.data.initial.items.find(
    (candidate) =>
      harness.data.initial.stores.find((store) => store.id === candidate.store)
        ?.server === harness.profile,
  );
  assert.ok(item);
  const recovered = harness.runtime().mutationError(
    {
      code: 'operation-failed',
      message: 'The write failed.',
      retryable: true,
      fatal: false,
      ambiguous: false,
    },
    { item, report: false },
  );
  await harness.clock.advance(0);
  await recovered;
  assert.deepEqual(harness.reads, [harness.profile]);
  assert.deepEqual(harness.whole, []);
});

test('a failed write with no item still reads the whole catalog back', async () => {
  const harness = await shell();
  const recovered = harness.runtime().mutationError(
    {
      code: 'operation-failed',
      message: 'The write failed.',
      retryable: true,
      fatal: false,
      ambiguous: false,
    },
    { report: false },
  );
  await harness.clock.advance(0);
  await recovered;
  assert.equal(harness.whole.length, 1);
  assert.deepEqual(harness.reads, []);
});

test('a failed post-mutation profile refresh reports its refresh error', async () => {
  const harness = await shell({
    override: (base) => ({
      ...base,
      listProfileCatalog: async (profile) => {
        await base.listProfileCatalog(profile);
        throw {
          code: 'operation-failed',
          message: 'Vault unavailable.',
          retryable: true,
          fatal: false,
          ambiguous: false,
        };
      },
    }),
  });
  const item = harness.data.initial.items.find(
    (candidate) =>
      harness.data.initial.stores.find((store) => store.id === candidate.store)
        ?.server === harness.profile,
  );
  assert.ok(item);
  const recovered = harness.runtime().mutationError(
    {
      code: 'operation-failed',
      message: 'The write failed.',
      retryable: true,
      fatal: false,
      ambiguous: false,
    },
    { item, report: false },
  );
  await harness.clock.advance(0);
  await recovered;
  // The read back failed, which the whole-catalog read would have reported
  // too; the write's own failure was asked not to be reported.
  assert.deepEqual(
    harness.runtime().reported.map((error) => (error as Error).message),
    ['Vault unavailable.'],
  );
});

test('a forced whole-catalog refresh discards cached metadata once, after its last partial', async () => {
  let emitted = 0;
  const harness = await shell({
    override: (base) => ({
      ...base,
      listCatalog: async (publish) => {
        const catalog = await base.listCatalog();
        for (let i = 0; i < 3; i++) {
          emitted++;
          publish?.(catalog);
        }
        return catalog;
      },
    }),
  });
  let invalidations = 0;
  const runtime = harness.runtime();
  runtime.catalog.metadataInvalidation.current = () => {
    invalidations++;
    // Invalidate metadata after all partial results publish so one subsequent
    // metadata read processes the completed catalog.
    assert.equal(emitted, 3);
  };
  await ui.act(async () => {
    await runtime.catalog.refreshSnapshot(true);
  });
  assert.equal(invalidations, 1);
  assert.equal(harness.runtime().catalog.hardwareRefresh, 1);
});
