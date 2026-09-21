import assert from 'node:assert/strict';
import test from 'node:test';
import { DesktopReconciliation } from '../src/desktop-reconciliation';
import { FIXTURE } from '../src/fixture';
import type { AgentSnapshot } from '../src/model';
import type { ReconciliationClock } from '../src/scheduling/reconciliation';

class Clock implements ReconciliationClock {
  time = 0;
  id = 0;
  tasks = new Map<number, { due: number; run: () => void }>();
  now = () => this.time;
  random = () => 0;
  later = (run: () => void, delay: number) => {
    const id = ++this.id;
    this.tasks.set(id, { due: this.time + delay, run });
    return id;
  };
  cancel = (id: unknown) => {
    this.tasks.delete(id as number);
  };
  async advance(ms: number) {
    const end = this.time + ms;
    for (;;) {
      const next = [...this.tasks].sort((a, b) => a[1].due - b[1].due)[0];
      if (!next || next[1].due > end) break;
      this.tasks.delete(next[0]);
      this.time = next[1].due;
      next[1].run();
      for (let i = 0; i < 30; i++) await Promise.resolve();
    }
    this.time = end;
  }
}
const lost = () => ({
  code: 'operation-failed',
  message: 'Offline',
  retryable: true,
  ambiguous: false,
  fatal: false,
});
const ready = (): AgentSnapshot => ({
  ...FIXTURE,
  accounts: [],
  servers: FIXTURE.servers.map((server) => ({
    ...server,
    trust: { status: 'verified' },
    restrictions: [],
  })),
});

function setup(
  connectivity: ConstructorParameters<
    typeof DesktopReconciliation
  >[0]['connectivity'],
) {
  const clock = new Clock();
  let snapshot = ready();
  let catalogs = 0;
  const service = new DesktopReconciliation(
    {
      snapshot: () => snapshot,
      nowSeconds: () => clock.now() / 1_000,
      connectivity,
      profile: async () => {
        catalogs++;
      },
      discovery: async () => {},
      registry: async () => {},
      metadata: async () => {},
    },
    clock,
  );
  service.update(snapshot);
  service.scheduler.setEnabled(true);
  return {
    service,
    clock,
    catalogs: () => catalogs,
    setSnapshot: (next: AgentSnapshot) => {
      snapshot = next;
      service.update(next);
    },
  };
}

test('startup checks saved profiles but ordinary catalog reads do not need a preceding probe', async () => {
  let probes = 0;
  const f = setup(async () => {
    probes++;
  });
  await f.clock.advance(0);
  assert.equal(probes, FIXTURE.catalogProfiles.length);
  const initial = probes;
  f.service.wake('foreground');
  await f.clock.advance(1_000);
  assert.equal(probes, initial);
  await f.clock.advance(30_000);
  assert.ok(f.catalogs() > 0);
  assert.equal(probes, initial);
  f.service.reconnect(FIXTURE.servers[0].id);
  await f.clock.advance(0);
  assert.equal(probes, initial + 1);
  f.service.dispose();
});

test('the result of a first observation does not run the observation again', async () => {
  const probes = new Map<string, number>();
  const f = setup(async (profile) => {
    probes.set(profile, (probes.get(profile) ?? 0) + 1);
  });
  const total = () => [...probes.values()].reduce((sum, n) => sum + n, 0);
  await f.clock.advance(0);
  assert.equal(total(), FIXTURE.catalogProfiles.length);
  const initial = total();
  // What a successful observation publishes: the profile's server identity
  // is bound and its trust follows from it. The job's schedule is derived
  // from the profile's configuration, so its own result does not restart it.
  f.setSnapshot({
    ...ready(),
    servers: ready().servers.map((server) => ({
      ...server,
      host_id: `${server.id}-bound`,
      trust: { status: 'verified' as const },
    })),
  });
  await f.clock.advance(30_000);
  assert.equal(total(), initial);
  assert.ok(f.catalogs() > 0);
  await f.clock.advance(120_000);
  assert.ok(total() > initial);
  f.service.dispose();
});

test('a reconfigured probe endpoint is observed without waiting for the interval', async () => {
  const probes = new Map<string, number>();
  const f = setup(async (profile) => {
    probes.set(profile, (probes.get(profile) ?? 0) + 1);
  });
  await f.clock.advance(0);
  const [reconfigured, unchanged] = FIXTURE.catalogProfiles;
  assert.equal(probes.get(reconfigured), 1);
  f.setSnapshot({
    ...ready(),
    servers: ready().servers.map((server) =>
      server.id === reconfigured
        ? { ...server, configuredProbe: 'foks.example.org' }
        : server,
    ),
  });
  await f.clock.advance(0);
  assert.equal(probes.get(reconfigured), 2);
  assert.equal(probes.get(unchanged), 1);
  f.service.dispose();
});

test('a failing profile backs off independently; focus storms do not bypass backoff', async () => {
  const failed = FIXTURE.servers[0].id;
  const calls = new Map<string, number>();
  const f = setup(async (profile) => {
    calls.set(profile, (calls.get(profile) ?? 0) + 1);
    if (profile === failed) throw lost();
  });
  await f.clock.advance(3_000);
  assert.equal(calls.get(failed), 3);
  for (let i = 0; i < 20; i++) f.service.wake('foreground');
  await f.clock.advance(3_000);
  assert.equal(calls.get(failed), 3);
  assert.equal(calls.get(FIXTURE.servers[1].id), 1);
  f.service.wake('network');
  await f.clock.advance(0);
  assert.equal(calls.get(failed), 4);
  f.service.dispose();
});

test('initial setup and security blocks cannot cause unattended trust creation', async () => {
  const called: string[] = [];
  const f = setup(async (profile) => {
    called.push(profile);
  });
  f.setSnapshot({
    ...ready(),
    stores: [],
    servers: ready().servers.map((server, i) => ({
      ...server,
      trust:
        i === 0 ? { status: 'unprobed' } : { status: 'blocked', error: lost() },
    })),
  });
  await f.clock.advance(120_000);
  assert.deepEqual(called, []);
  f.service.dispose();
});

test('lock and profile removal retire an outstanding connection result', async () => {
  let finish!: () => void;
  const pending = new Promise<void>((resolve) => {
    finish = resolve;
  });
  let published = 0;
  const f = setup(async (_profile, context) => {
    await pending;
    if (context.isCurrent()) published++;
  });
  await f.clock.advance(0);
  f.service.scheduler.setEnabled(false);
  f.setSnapshot({ ...ready(), stores: [], servers: [], catalogProfiles: [] });
  finish();
  await f.clock.advance(0);
  assert.equal(published, 0);
  f.service.dispose();
});
