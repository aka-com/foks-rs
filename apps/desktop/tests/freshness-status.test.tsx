import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer, type ViteDevServer } from 'vite';
import { metadataFreshness } from '../src/device-cache';
import {
  DesktopReconciliation,
  profileConnectivityKey,
  profileRefreshKey,
} from '../src/desktop-reconciliation';
import { FIXTURE } from '../src/fixture';
import {
  failWholeCatalogRefresh,
  markCatalogRefresh,
} from '../src/catalog-state';
let FreshnessCaption: typeof import('../src/components/metadata-status').FreshnessCaption;
let summarizeSync: typeof import('../src/shell/sync-popover').summarizeSync;
let vite: ViteDevServer;
test.before(async () => {
  vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
  ({ FreshnessCaption } = await vite.ssrLoadModule(
    '/src/components/metadata-status.tsx',
  ));
  ({ summarizeSync } = await vite.ssrLoadModule('/src/shell/sync-popover.tsx'));
});
test.after(async () => {
  await vite.close();
});

test('the freshness caption names a retained failure, spins while loading, and is silent when fresh', () => {
  const data = {
    data: [],
    error: undefined,
    fetching: false,
    invalidation: 0,
    lastSuccessAt: 1000,
  };
  const failed = metadataFreshness([
    data,
    { ...data, error: new Error('offline'), lastSuccessAt: 2000 },
  ]);
  assert.equal(failed.lastSuccessAt, 1000);
  assert.equal(failed.stale, true);
  const html = renderToStaticMarkup(
    createElement(FreshnessCaption, {
      label: 'device metadata',
      freshness: failed,
      onRetry: () => {},
    }),
  );
  assert.match(html, /class="freshness stale"/);
  assert.match(html, /As of .* · device metadata could not be refreshed/);
  assert.match(html, />Retry</);
  // Nothing was ever read, and the read failed: no time to quote.
  const never = metadataFreshness([
    { ...data, data: undefined, lastSuccessAt: undefined, error: new Error() },
  ]);
  assert.match(
    renderToStaticMarkup(
      createElement(FreshnessCaption, {
        label: 'device metadata',
        freshness: never,
      }),
    ),
    /Device metadata could not be loaded/,
  );
  // A refresh in progress is a spinner, not a sentence.
  const loading = metadataFreshness([
    { ...data, data: undefined, lastSuccessAt: undefined, fetching: true },
  ]);
  assert.equal(loading.lastSuccessAt, undefined);
  const spinning = renderToStaticMarkup(
    createElement(FreshnessCaption, {
      label: 'device metadata',
      freshness: loading,
    }),
  );
  assert.match(spinning, /class="freshness refreshing"/);
  assert.match(spinning, /aria-label="device metadata refreshing"/);
  assert.doesNotMatch(spinning, /Refreshing/);
  // Fresh data draws nothing at all.
  assert.equal(
    renderToStaticMarkup(
      createElement(FreshnessCaption, {
        label: 'device metadata',
        freshness: metadataFreshness([data]),
      }),
    ),
    '',
  );
});

test('background metadata refreshes stay quiet only when every query has data', () => {
  const cached = {
    data: [],
    error: undefined,
    fetching: true,
    invalidation: 0,
    lastSuccessAt: 1000,
  };
  const render = (freshness: ReturnType<typeof metadataFreshness>) =>
    renderToStaticMarkup(
      createElement(FreshnessCaption, {
        label: 'device metadata',
        freshness,
        onRetry: () => {},
      }),
    );
  assert.equal(render(metadataFreshness([cached, cached])), '');
  assert.match(
    render(
      metadataFreshness([
        cached,
        { ...cached, data: undefined, lastSuccessAt: undefined },
      ]),
    ),
    /class="freshness refreshing"/,
  );
  const failed = render(
    metadataFreshness([
      cached,
      { ...cached, fetching: false, error: new Error('offline') },
    ]),
  );
  assert.match(failed, /device metadata could not be refreshed/);
  assert.match(failed, />Retry</);
});

const FAILURE = {
  code: 'operation-failed',
  message: 'Keystore record is missing',
  retryable: true,
  fatal: false,
  ambiguous: false,
};

function service(snapshot: typeof FIXTURE) {
  return new DesktopReconciliation({
    snapshot: () => snapshot,
    nowSeconds: () => 20,
    profile: async () => {},
    registry: async () => {},
    discovery: async () => {},
    metadata: async () => {},
  });
}

test('the refresh summary does not call a failed observation a successful refresh', () => {
  const profile = FIXTURE.servers[0].id;
  const snapshot = {
    ...FIXTURE,
    catalogFreshness: {
      stores: {},
      profiles: {
        [profile]: {
          refreshing: false,
          lastAttemptAt: 20,
          lastSuccessAt: 10,
          error: { ...FAILURE, message: 'Server unavailable' },
        },
      },
    },
  };
  const reconciliation = service(snapshot);
  const summary = summarizeSync(snapshot, reconciliation);
  assert.equal(summary.failed, true);
  const row = summary.servers.find((server) => server.id === profile);
  assert.equal(row?.state, 'failed');
  assert.match(row?.message ?? '', /^Server unavailable\. Showing data from /);
  assert.match(row?.message ?? '', /Retrying automatically\.$/);
  // The old per-observation line survives for Copy diagnostics.
  assert.ok(
    summary.diagnostics.some((line) =>
      /Last successful refresh: .*Refresh failed: Server unavailable/.test(
        line,
      ),
    ),
  );
  reconciliation.dispose();
});

test('whole-catalog failures have one root observation without marking healthy servers failed', () => {
  const previous = {
    ...FIXTURE,
    catalogFreshness: {
      profiles: Object.fromEntries(
        FIXTURE.catalogProfiles.map((profile) => [
          profile,
          { refreshing: false, lastSuccessAt: 10 },
        ]),
      ),
      stores: {},
    },
  };
  const snapshot = failWholeCatalogRefresh(
    markCatalogRefresh(previous, undefined, 20),
    FAILURE,
    21,
  );
  const reconciliation = service(snapshot);
  const summary = summarizeSync(snapshot, reconciliation);
  const failed = summary.servers.filter((row) => row.state === 'failed');
  assert.equal(failed.length, 1);
  assert.equal(failed[0].id, null);
  assert.equal(failed[0].name, 'Catalog');
  assert.match(failed[0].message, /Use Refresh to retry/);
  assert.doesNotMatch(
    failed[0].message,
    /Retrying automatically|Automatic refresh is paused/,
  );
  assert.deepEqual(
    summary.servers
      .filter((row) => row.id !== null)
      .map((row) => [row.id, row.state]),
    summarizeSync(previous, reconciliation).servers.map((row) => [
      row.id,
      row.state,
    ]),
  );
  assert.equal(
    summary.diagnostics.filter((line) => line.includes(FAILURE.message)).length,
    1,
  );
  reconciliation.dispose();
});

test('a root refresh remains observable before any server inventory is available', () => {
  const empty = { ...FIXTURE, servers: [], stores: [], catalogProfiles: [] };
  const pending = markCatalogRefresh(empty, undefined, 20);
  const reconciliation = service(pending);
  assert.equal(summarizeSync(pending, reconciliation).refreshing, true);
  const failed = failWholeCatalogRefresh(pending, FAILURE, 21);
  const summary = summarizeSync(failed, reconciliation);
  assert.equal(summary.refreshing, false);
  assert.equal(summary.failed, true);
  assert.equal(summary.servers.length, 1);
  assert.doesNotMatch(summary.servers[0].message, /Previously loaded data/);
  reconciliation.dispose();
});

test('refresh status uses the scheduler paused state', () => {
  const profile = FIXTURE.servers[0].id;
  const fatal = {
    code: 'response-binding',
    message: 'The agent’s reply did not match the request',
    retryable: false,
    fatal: true,
    ambiguous: false,
  };
  // The catalog job failed fatally: the catalog state carries the error, as
  // it does on every failure, and the scheduler parked the job.
  const parked = {
    ...FIXTURE,
    catalogFreshness: {
      stores: {},
      profiles: {
        [profile]: {
          refreshing: false,
          lastAttemptAt: 20,
          lastSuccessAt: 10,
          error: fatal,
        },
      },
    },
  };
  const scheduler = (
    observations: {
      key: string;
      scope: string | null;
      kind: 'catalog' | 'metadata';
      snapshot: {
        refreshing: boolean;
        lastAttemptAt: number;
        error: unknown;
        paused: boolean;
      };
    }[],
  ) =>
    ({
      supportsConnectivity: true,
      scheduler: {
        observations: () => observations,
        snapshot: (key: string) =>
          observations.find((observation) => observation.key === key)?.snapshot,
      },
    }) as unknown as DesktopReconciliation;
  const catalogParked = summarizeSync(
    parked,
    scheduler([
      {
        key: profileRefreshKey(parked, profile),
        scope: profile,
        kind: 'catalog',
        snapshot: {
          refreshing: false,
          lastAttemptAt: 20,
          error: fatal,
          paused: true,
        },
      },
    ]),
  );
  const row = catalogParked.servers.find((server) => server.id === profile);
  assert.match(row?.message ?? '', /Automatic refresh is paused/);
  assert.doesNotMatch(row?.message ?? '', /Retrying automatically/);
  // The metadata job's failure was as fatal, but the scheduler keeps retrying
  // metadata, so its sentence says so.
  const metadataRetrying = summarizeSync(
    FIXTURE,
    scheduler([
      {
        key: 'metadata',
        scope: null,
        kind: 'metadata',
        snapshot: {
          refreshing: false,
          lastAttemptAt: 20,
          error: fatal,
          paused: false,
        },
      },
    ]),
  );
  const local = metadataRetrying.servers.find((server) => server.id === null);
  assert.equal(local?.name, 'This Mac');
  assert.match(local?.message ?? '', /Retrying automatically/);
  assert.doesNotMatch(local?.message ?? '', /paused/);
});

test('observations that share one root cause on one server collapse to one row and one sentence', () => {
  const profile = FIXTURE.servers[0].id;
  const snapshot = {
    ...FIXTURE,
    catalogFreshness: {
      stores: {},
      profiles: {
        [profile]: {
          refreshing: false,
          lastAttemptAt: 20,
          lastSuccessAt: 10,
          error: FAILURE,
        },
      },
    },
  };
  // Four jobs failed on the one server, and one on this Mac, all with the same
  // message: the catalog read, connectivity, discovery and profile inventory.
  const observations = [
    { key: 'catalog', scope: profile, kind: 'catalog' as const },
    { key: 'connectivity', scope: profile, kind: 'connectivity' as const },
    { key: 'discovery', scope: profile, kind: 'discovery' as const },
    { key: 'registry', scope: null, kind: 'registry' as const },
  ].map((job) => ({
    ...job,
    snapshot: { refreshing: false, lastAttemptAt: 20, error: FAILURE },
  }));
  const fake = {
    supportsConnectivity: true,
    scheduler: {
      observations: () => observations,
      snapshot: (key: string) =>
        observations.find((observation) => observation.key === key)?.snapshot,
    },
  } as unknown as DesktopReconciliation;
  const summary = summarizeSync(snapshot, fake);
  const rows = summary.servers.filter((server) => server.id === profile);
  assert.equal(rows.length, 1);
  assert.equal(
    (rows[0].message.match(/Keystore record is missing/g) ?? []).length,
    1,
  );
  assert.match(
    rows[0].message,
    /^Keystore record is missing\. Showing data from .*\. Retrying automatically\.$/,
  );
  assert.equal(rows[0].state, 'failed');
  assert.equal(summary.failed, true);
  // The registry job belongs to no server, so it is its own row.
  const local = summary.servers.find((server) => server.id === null);
  assert.equal(local?.name, 'This Mac');
  assert.match(local?.message ?? '', /^Keystore record is missing\. Retrying/);
  // Every failed observation still has a diagnostic line of its own.
  assert.equal(
    summary.diagnostics.filter((line) =>
      /Keystore record is missing/.test(line),
    ).length,
    4,
  );
});

test('each row lists its jobs: when they ran, when they run next, and what failed', () => {
  const profile = FIXTURE.servers[0].id;
  const scheduler = (
    observations: {
      key: string;
      scope: string | null;
      kind: 'catalog' | 'connectivity' | 'metadata';
      snapshot: {
        refreshing: boolean;
        lastAttemptAt?: number;
        lastSuccessAt?: number;
        error?: unknown;
        paused?: boolean;
        nextAttemptAt?: number;
      };
    }[],
  ) =>
    ({
      supportsConnectivity: true,
      scheduler: {
        observations: () => observations,
        snapshot: (key: string) =>
          observations.find((observation) => observation.key === key)?.snapshot,
      },
    }) as unknown as DesktopReconciliation;
  const summary = summarizeSync(
    FIXTURE,
    scheduler([
      {
        key: profileConnectivityKey(FIXTURE, profile),
        scope: profile,
        kind: 'connectivity',
        snapshot: { refreshing: true, lastAttemptAt: 21_000 },
      },
      {
        key: profileRefreshKey(FIXTURE, profile),
        scope: profile,
        kind: 'catalog',
        snapshot: {
          refreshing: false,
          lastAttemptAt: 20_000,
          lastSuccessAt: 20_000,
          nextAttemptAt: 50_000,
        },
      },
      {
        key: 'metadata',
        scope: null,
        kind: 'metadata',
        snapshot: {
          refreshing: false,
          lastAttemptAt: 20_000,
          error: {
            code: 'io',
            message: 'Metadata read failed.',
            retryable: true,
            fatal: false,
            ambiguous: false,
          },
          paused: false,
          nextAttemptAt: 30_000,
        },
      },
    ]),
  );
  const row = summary.servers.find((server) => server.id === profile);
  // The catalog is listed before the jobs that keep it, whatever order the
  // scheduler holds them in.
  assert.deepEqual(
    row?.jobs.map((job) => [job.label, job.state]),
    [
      ['Catalog', 'ok'],
      ['Connectivity reconciliation', 'refreshing'],
    ],
  );
  assert.match(row?.jobs[0].detail ?? '', /^Succeeded .+\. Next at .+\.$/);
  assert.equal(row?.jobs[1].detail, 'Running now.');
  const local = summary.servers.find((server) => server.id === null);
  assert.deepEqual(
    local?.jobs.map((job) => [job.label, job.state]),
    [['Account metadata', 'failed']],
  );
  assert.match(local?.jobs[0].detail ?? '', /^Metadata read failed\. Next at .+\.$/);
  // A parked job says so instead of naming a next time.
  const parked = summarizeSync(
    FIXTURE,
    scheduler([
      {
        key: profileRefreshKey(FIXTURE, profile),
        scope: profile,
        kind: 'catalog',
        snapshot: {
          refreshing: false,
          lastAttemptAt: 20_000,
          lastSuccessAt: 10_000,
          error: {
            code: 'response-binding',
            message: 'The reply did not match.',
            retryable: false,
            fatal: true,
            ambiguous: false,
          },
          paused: true,
        },
      },
    ]),
  );
  assert.equal(
    parked.servers.find((server) => server.id === profile)?.jobs[0].detail,
    `The reply did not match. Last succeeded ${new Date(10_000).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })}. Paused until Refresh.`,
  );
});
