import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { Bridge } from '../src/bridge';
import type { FileUploads } from '../src/bridge/uploads';
import { installDom } from './lib/dom-harness';
installDom({ url: 'http://localhost/', body: '<div id="root"></div>' });
let ui: typeof import('@testing-library/react');
test.before(async () => {
  ui = await import('@testing-library/react');
});
test.afterEach(() => ui.cleanup());

test('a rejected publication offers Resume and Cancel', async () => {
  const { FileUploadsPanel } =
    await import('../src/features/files/file-uploads');
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
  ui.render(createElement(FileUploadsPanel, { bridge, storeId: 'store' }));
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Uploads' }));
  await ui.screen.findByText(/Publication rejected; resume to publish again/);
  ui.fireEvent.click(ui.screen.getByRole('button', { name: /Resume/ }));
  await ui.waitFor(() => {
    assert.deepEqual(calls, [`resume store ${uploadId}`]);
  });
  const cancel = await ui.screen.findByRole('button', {
    name: 'Cancel upload',
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
