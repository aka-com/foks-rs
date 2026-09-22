import assert from 'node:assert/strict';
import test from 'node:test';
import {
  changedCatalogProfiles,
  failCatalogRefresh,
  failWholeCatalogRefresh,
  markCatalogRefresh,
  mergeProfileSnapshot,
  projectCatalogFreshness,
} from '../src/catalog-state';
import { FIXTURE } from '../src/fixture';
import type { AgentSnapshot } from '../src/model';
import { mockBridge } from '../src/mock-bridge';

test('concurrent profile publication merges only its own scope into the newest base', () => {
  const [a, b] = FIXTURE.servers;
  assert.ok(a && b);
  const earlier = {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server) =>
      server.profileName === a.profileName
        ? { ...server, displayLabel: 'A refreshed' }
        : server,
    ),
  };
  const later = {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server) =>
      server.profileName === b.profileName
        ? { ...server, displayLabel: 'B refreshed' }
        : server,
    ),
  };
  const result = mergeProfileSnapshot(later, earlier, a.profileName);
  assert.equal(result.servers.length, FIXTURE.servers.length);
  assert.equal(
    result.servers.find((server) => server.profileName === a.profileName)
      ?.displayLabel,
    'A refreshed',
  );
  assert.equal(
    result.servers.find((server) => server.profileName === b.profileName)
      ?.displayLabel,
    'B refreshed',
  );
  assert.equal(result.stores.length, FIXTURE.stores.length);
  assert.equal(result.items.length, FIXTURE.items.length);
});

test('a whole-read failure preserves successful partial observations and prior scoped failures', async () => {
  const response = await mockBridge(FIXTURE).listCatalog();
  const [healthy, failed] = FIXTURE.servers;
  const scoped = {
    code: 'server-unavailable',
    message: 'One server unavailable.',
    retryable: true,
    fatal: false,
    ambiguous: false,
  };
  const root = {
    ...scoped,
    code: 'invalid-response',
    message: 'Catalog projection failed.',
  };
  const base = {
    ...FIXTURE,
    catalogFreshness: projectCatalogFreshness(
      FIXTURE,
      undefined,
      response,
      false,
      10,
    ),
  };
  const failedProfile = failCatalogRefresh(
    base,
    [failed.profileName],
    scoped,
    20,
  );
  const started = markCatalogRefresh(failedProfile, undefined, 30);
  assert.strictEqual(
    started.catalogFreshness?.profiles[failed.profileName].error,
    scoped,
  );
  const pending = projectCatalogFreshness(
    started,
    started,
    { ...response, fullItemReads: [] },
    true,
    30,
  );
  assert.strictEqual(pending.profiles[failed.profileName].error, scoped);
  const partial = {
    ...started,
    catalogFreshness: {
      ...started.catalogFreshness,
      profiles: {
        ...started.catalogFreshness.profiles,
        [healthy.profileName]: {
          refreshing: false,
          lastAttemptAt: 30,
          lastSuccessAt: 30,
        },
      },
    },
  };
  const terminal = failWholeCatalogRefresh(partial, root, 31);
  assert.strictEqual(terminal.catalogFreshness?.attempt?.error, root);
  assert.equal(terminal.catalogFreshness?.attempt?.lastSuccessAt, 10);
  assert.strictEqual(
    terminal.catalogFreshness?.profiles[healthy.profileName],
    partial.catalogFreshness.profiles[healthy.profileName],
  );
  assert.strictEqual(
    terminal.catalogFreshness?.profiles[failed.profileName].error,
    scoped,
  );
  assert.ok(
    Object.values(terminal.catalogFreshness.stores).every(
      (entry) => entry.error !== root && !entry.refreshing,
    ),
  );
  assert.strictEqual(terminal.notifications, base.notifications);
  assert.strictEqual(terminal.storeInventory, base.storeInventory);
  assert.strictEqual(terminal.items, base.items);
  const scopedSuccess = mergeProfileSnapshot(
    terminal,
    base,
    healthy.profileName,
  );
  assert.strictEqual(
    scopedSuccess.catalogFreshness?.attempt,
    terminal.catalogFreshness?.attempt,
  );
  assert.strictEqual(
    markCatalogRefresh(terminal, [healthy.profileName], 32).catalogFreshness
      ?.attempt,
    terminal.catalogFreshness?.attempt,
  );
});

test('only a completed whole read advances whole-catalog success', async () => {
  const response = await mockBridge(FIXTURE).listCatalog();
  const error = {
    code: 'io',
    message: 'Read failed.',
    retryable: false,
    fatal: false,
    ambiguous: false,
  };
  const base = failWholeCatalogRefresh(FIXTURE, error, 10);
  const partial = projectCatalogFreshness(base, base, response, true, 20);
  assert.equal(partial.attempt?.refreshing, true);
  assert.equal(partial.attempt?.lastSuccessAt, undefined);
  const scoped = projectCatalogFreshness(
    base,
    base,
    response,
    false,
    20,
    response.profiles[0],
  );
  assert.equal(scoped.attempt, undefined);
  const final = projectCatalogFreshness(base, base, response, false, 20);
  assert.equal(final.attempt?.refreshing, false);
  assert.equal(final.attempt?.lastSuccessAt, 20);
  assert.equal(final.attempt?.error, undefined);
});

test('retained store content does not count as a newly completed read', async () => {
  const response = await mockBridge(FIXTURE).listCatalog();
  const store = FIXTURE.stores[0];
  const base = {
    ...FIXTURE,
    catalogFreshness: {
      profiles: {},
      stores: { [store.id]: { refreshing: false, lastSuccessAt: 10 } },
    },
  };
  const freshness = projectCatalogFreshness(
    base,
    base,
    {
      ...response,
      fullItemReads: [],
      storeReads: [{ store: store.id, state: 'not-loaded' }],
    },
    true,
    20,
  );
  assert.equal(freshness.stores[store.id].lastSuccessAt, 10);
  assert.equal(freshness.stores[store.id].refreshing, true);
  const complete = projectCatalogFreshness(
    base,
    base,
    {
      ...response,
      fullItemReads: [],
      storeReads: [{ store: store.id, state: 'complete' }],
    },
    true,
    30,
  );
  assert.equal(complete.stores[store.id].lastSuccessAt, 30);
});

function withFreshness(
  snapshot: AgentSnapshot,
  lastSuccessAt = 1,
): AgentSnapshot {
  return {
    ...snapshot,
    catalogFreshness: {
      profiles: Object.fromEntries(
        snapshot.catalogProfiles.map((profile) => [
          profile,
          { refreshing: false, lastAttemptAt: lastSuccessAt, lastSuccessAt },
        ]),
      ),
      stores: {},
    },
  };
}

test('a forced refresh with no comparable previous snapshot names no profiles', () => {
  const next = withFreshness(FIXTURE);
  assert.equal(changedCatalogProfiles(undefined, next), undefined);
  assert.equal(changedCatalogProfiles(FIXTURE, next), undefined);
  assert.equal(changedCatalogProfiles(next, FIXTURE), undefined);
});

test('only the profiles whose stores or accounts changed are named', () => {
  const [a, b] = FIXTURE.catalogProfiles;
  assert.ok(a && b);
  const previous = withFreshness(FIXTURE, 1);
  // A later read of the same catalog: every profile's attempt and success
  // times move, and none of its content does.
  assert.deepEqual(
    changedCatalogProfiles(previous, withFreshness(FIXTURE, 2)),
    [],
  );
  const dropped = {
    ...FIXTURE,
    stores: FIXTURE.stores.filter((store) => store.server !== a),
  };
  assert.deepEqual(
    changedCatalogProfiles(previous, withFreshness(dropped, 2)),
    [a],
  );
  const renamed = {
    ...FIXTURE,
    accounts: FIXTURE.accounts.map((account) =>
      account.server === b ? { ...account, alias: 'other' } : account,
    ),
  };
  assert.deepEqual(
    changedCatalogProfiles(previous, withFreshness(renamed, 2)),
    [b],
  );
  // A display label is not metadata anything is keyed on.
  const relabelled = {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server) =>
      server.profileName === a
        ? { ...server, displayLabel: 'Renamed' }
        : server,
    ),
  };
  assert.deepEqual(
    changedCatalogProfiles(previous, withFreshness(relabelled, 2)),
    [],
  );
});

test('a profile whose freshness entry is missing counts as changed', () => {
  const [a] = FIXTURE.catalogProfiles;
  assert.ok(a);
  const previous = withFreshness(FIXTURE);
  const next = withFreshness(FIXTURE, 2);
  const missing: AgentSnapshot = {
    ...next,
    catalogFreshness: {
      ...next.catalogFreshness!,
      profiles: Object.fromEntries(
        Object.entries(next.catalogFreshness!.profiles).filter(
          ([profile]) => profile !== a,
        ),
      ),
    },
  };
  assert.deepEqual(changedCatalogProfiles(previous, missing), [a]);
  assert.deepEqual(changedCatalogProfiles(missing, next), [a]);
});

test('a profile that started or stopped failing is named', () => {
  const [a] = FIXTURE.catalogProfiles;
  assert.ok(a);
  const previous = withFreshness(FIXTURE);
  const next = withFreshness(FIXTURE, 2);
  const failed: AgentSnapshot = {
    ...next,
    catalogFreshness: {
      ...next.catalogFreshness!,
      profiles: {
        ...next.catalogFreshness!.profiles,
        [a]: {
          ...next.catalogFreshness!.profiles[a],
          error: {
            code: 'server-unavailable',
            message: 'away',
            retryable: true,
            fatal: false,
            ambiguous: false,
          },
        },
      },
    },
  };
  assert.deepEqual(changedCatalogProfiles(previous, failed), [a]);
  assert.deepEqual(changedCatalogProfiles(failed, next), [a]);
});
