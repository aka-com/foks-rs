import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer, type ViteDevServer } from 'vite';
import { metadataFreshness } from '../src/device-cache';
import { DesktopReconciliation } from '../src/desktop-reconciliation';
import { FIXTURE } from '../src/fixture';
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
