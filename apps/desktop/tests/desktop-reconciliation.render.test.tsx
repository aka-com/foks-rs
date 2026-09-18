import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, useRef, useState } from 'react';
import { installDom } from './lib/dom-harness';
import { useDesktopReconciliation } from '../src/use-desktop-reconciliation';
import { CatalogReadGate } from '../src/catalog-read-gate';
import { mockBridge } from '../src/mock-bridge';
import { FIXTURE } from '../src/fixture';
import type { Bridge, CatalogDto } from '../src/bridge';
import { serverAvailability, type AgentSnapshot } from '../src/model';
import type { ReconciliationClock } from '../src/scheduling/reconciliation';

installDom({ url: 'http://localhost/', timers: true, act: true });
let ui: typeof import('@testing-library/react');
test.before(async () => {
  ui = await import('@testing-library/react');
});
test.afterEach(() => ui.cleanup());
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
}: {
  bridge: Bridge;
  initial: AgentSnapshot;
  clock: Clock;
  enabled?: boolean;
  publish(this: void, snapshot: AgentSnapshot): void;
}) {
  const [snapshot, setSnapshot] = useState(initial);
  const latest = useRef(snapshot);
  latest.current = snapshot;
  const [gate] = useState(() => new CatalogReadGate());
  useDesktopReconciliation({
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
