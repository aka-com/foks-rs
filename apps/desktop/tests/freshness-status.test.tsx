import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer, type ViteDevServer } from 'vite';
import { metadataFreshness } from '../src/device-cache';
import { DesktopReconciliation } from '../src/desktop-reconciliation';
import { FIXTURE } from '../src/fixture';
let MetadataStatus: typeof import('../src/components/metadata-status').MetadataStatus;
let SyncStatus: typeof import('../src/shell/sync-status').SyncStatus;
let vite: ViteDevServer;
test.before(async () => {
  vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
  ({ MetadataStatus } = await vite.ssrLoadModule(
    '/src/components/metadata-status.tsx',
  ));
  ({ SyncStatus } = await vite.ssrLoadModule('/src/shell/sync-status.tsx'));
});
test.after(async () => {
  await vite.close();
});

test('metadata freshness distinguishes retained failures from successful reads', () => {
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
    createElement(MetadataStatus, { label: 'Devices', freshness: failed }),
  );
  assert.match(html, /Refresh failed; showing previously loaded data/);
  assert.match(html, /Last successful read/);
  const loading = metadataFreshness([
    { ...data, data: undefined, lastSuccessAt: undefined, fetching: true },
  ]);
  assert.equal(loading.lastSuccessAt, undefined);
  assert.match(
    renderToStaticMarkup(
      createElement(MetadataStatus, { label: 'Devices', freshness: loading }),
    ),
    /Refreshing/,
  );
});

test('catalog freshness status does not call a failed observation a successful refresh', () => {
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
          error: {
            code: 'operation-failed',
            message: 'Server unavailable',
            retryable: true,
            fatal: false,
            ambiguous: false,
          },
        },
      },
    },
  };
  const service = new DesktopReconciliation({
    snapshot: () => snapshot,
    nowSeconds: () => 20,
    profile: async () => {},
    registry: async () => {},
    discovery: async () => {},
    metadata: async () => {},
  });
  const html = renderToStaticMarkup(
    createElement(SyncStatus, { snapshot, service }),
  );
  assert.match(html, /Some data could not be refreshed/);
  assert.match(html, /Refresh failed: Server unavailable/);
  assert.match(html, /Last successful refresh/);
  service.dispose();
});
