import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import type { Bridge } from '../src/bridge';
import type { FileUploads } from '../src/bridge/uploads';
import { installDom } from './lib/dom-harness';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
});
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

/** The sheet portals into the overlay root, so the panel needs a provider. */
async function overlay(child: ReturnType<typeof createElement>) {
  const { OverlayProvider } = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives');
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  return createElement(OverlayProvider, {
    backgroundRef: { current: null },
    portalRoot,
    children: child,
  });
}

test('a rejected publication offers Resume and Cancel Upload', async () => {
  const { FileUploadsPanel } = (await vite.ssrLoadModule(
    '/src/screens/file-uploads.tsx',
  )) as typeof import('../src/screens/file-uploads');
  const uploadId = '01'.repeat(16);
  const inventory: FileUploads = {
    uploads: [
      {
        uploadId,
        state: 'publication-rejected',
        preparedChunks: 1,
        acknowledgedChunks: 1,
        plaintextBytes: 3000,
        reservedBytes: 9000,
        nodeId: null,
        version: null,
      },
    ],
    next: null,
    capacity: {
      maxReservedBytes: 268435456,
      maxPending: 16,
      maxReceipts: 8192,
      maxReceiptBytes: 16777216,
    },
  };
  const calls: string[] = [];
  const bridge = {
    listFileUploads: () => Promise.resolve(inventory),
    resumeFileUpload: (store: string, id: string) => {
      calls.push(`resume ${store} ${id}`);
      return Promise.resolve();
    },
    cancelFileUpload: (store: string, id: string) => {
      calls.push(`cancel ${store} ${id}`);
      return Promise.resolve();
    },
  } as unknown as Bridge;
  ui.render(
    await overlay(
      createElement(FileUploadsPanel, {
        bridge,
        storeId: 'store',
        storeName: 'Personal',
      }),
    ),
  );
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Uploads…' }));
  await ui.screen.findByText('Publication rejected; resume to publish again');
  const resume = await ui.screen.findByRole('button', { name: 'Resume' });
  await ui.waitFor(() => {
    assert.equal((resume as HTMLButtonElement).disabled, false);
  });
  ui.fireEvent.click(resume);
  await ui.waitFor(() => {
    assert.deepEqual(calls, [`resume store ${uploadId}`]);
  });
  const cancel = await ui.screen.findByRole('button', {
    name: 'Cancel Upload',
  });
  await ui.waitFor(() => {
    assert.equal((cancel as HTMLButtonElement).disabled, false);
  });
  ui.fireEvent.click(cancel);
  await ui.waitFor(() => {
    assert.deepEqual(calls, [
      `resume store ${uploadId}`,
      `cancel store ${uploadId}`,
    ]);
  });
});
