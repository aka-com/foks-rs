/**
 * The Uploads panel: the actions each upload state offers, the saved
 * receipt of a finished upload, a resume or cancel that refreshes the
 * store's server and keeps the page the list showed.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';

import type { Bridge } from '../src/bridge';
import type { FileUpload, FileUploads } from '../src/bridge/uploads';
import { installDom } from './lib/dom-harness';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div>',
  timers: true,
});

let ui: typeof import('@testing-library/react');
test.before(async () => {
  ui = await import('@testing-library/react');
});
test.afterEach(() => ui.cleanup());

const STORE = 'acct:personal';
const SECOND_PAGE = Array.from({ length: 32 }, (_, index) => index);

function upload(index: number, state: string): FileUpload {
  const done = ['verified', 'completed'].includes(state);
  return {
    uploadId: index.toString(16).padStart(2, '0').repeat(16),
    state,
    preparedChunks: 2,
    acknowledgedChunks: done ? 2 : 1,
    plaintextBytes: 1024,
    reservedBytes: 4096,
    nodeId: done ? `node-${index}` : null,
    version: done ? 7 : null,
  };
}

const STATES = [
  'awaiting-source',
  'inline-untracked',
  'needs-source',
  'ready',
  'uploading',
  'content-complete',
  'publishing',
  'verified',
  'completed',
  'blocked',
  'cancelled',
];

function page(uploads: FileUpload[], next: number[] | null): FileUploads {
  return {
    uploads,
    next,
    capacity: {
      maxReservedBytes: 1 << 28,
      maxPending: 16,
      maxReceipts: 8192,
      maxReceiptBytes: 1 << 24,
    },
  };
}

async function mount(resumeFailure?: { code: string; message: string }) {
  const { FileUploadsPanel } =
    await import('../src/features/files/file-uploads');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const calls: string[] = [];
  const refreshed: string[] = [];
  const first = page(
    STATES.map((state, index) => upload(index, state)),
    SECOND_PAGE,
  );
  const second = page([upload(20, 'ready')], null);
  const bridge: Bridge = {
    ...mockBridge(),
    listFileUploads: async (storeId, after) => {
      assert.equal(storeId, STORE);
      calls.push(`list ${after ? 'second' : 'first'}`);
      return after ? second : first;
    },
    resumeFileUpload: async (_storeId, uploadId) => {
      calls.push(`resume ${uploadId}`);
      if (resumeFailure)
        throw {
          ...resumeFailure,
          retryable: false,
          ambiguous: false,
          fatal: false,
        };
      return upload(20, 'completed');
    },
    cancelFileUpload: async (_storeId, uploadId) => {
      calls.push(`cancel ${uploadId}`);
      return upload(20, 'cancelled');
    },
  };
  ui.render(
    createElement(FileUploadsPanel, {
      bridge,
      storeId: STORE,
      onChanged: async (message: string) => {
        refreshed.push(message);
      },
    }),
    { container: document.getElementById('root')! },
  );
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Uploads' }));
  await ui.screen.findByText(upload(0, STATES[0]).uploadId);
  return { calls, refreshed };
}

/** The labels of the buttons on the row of `state`. */
function actions(state: string): string[] {
  const index = STATES.indexOf(state);
  const row = ui.screen.getByText(upload(index, state).uploadId).closest('li')!;
  return [...row.querySelectorAll('button')].map((button) =>
    button.textContent.trim(),
  );
}

test('each upload state offers only the actions the agent can take', async () => {
  await mount();
  const expected: Record<string, string[]> = {
    'awaiting-source': ['Cancel upload'],
    'inline-untracked': ['Clean up local spool'],
    'needs-source': ['Cancel upload'],
    ready: ['Resume', 'Cancel upload'],
    uploading: ['Resume', 'Cancel upload'],
    'content-complete': ['Resume', 'Cancel upload'],
    publishing: ['Resume'],
    verified: ['Resume'],
    completed: ['Clean up local spool'],
    blocked: ['Resume', 'Cancel upload'],
    cancelled: ['Clean up local spool'],
  };
  for (const state of STATES)
    assert.deepEqual(actions(state), expected[state], state);
});

test('a finished upload shows the node and version it was saved as', async () => {
  await mount();
  for (const state of ['verified', 'completed']) {
    const index = STATES.indexOf(state);
    const row = ui.screen
      .getByText(upload(index, state).uploadId)
      .closest('li')!;
    assert.match(row.textContent, new RegExp(`saved as node node-${index}`));
    assert.match(row.textContent, /version 7/);
  }
  const ready = ui.screen
    .getByText(upload(STATES.indexOf('ready'), 'ready').uploadId)
    .closest('li')!;
  assert.doesNotMatch(ready.textContent, /saved as/);
});

test('a resume or cancel refreshes the server and keeps the page', async () => {
  const { calls, refreshed } = await mount();
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Next uploads' }));
  const second = upload(20, 'ready').uploadId;
  await ui.screen.findByText(second);
  const row = () => ui.screen.getByText(second).closest('li')!;
  ui.fireEvent.click(ui.within(row()).getByRole('button', { name: 'Resume' }));
  await ui.waitFor(() => assert.equal(calls.length, 4));
  ui.fireEvent.click(
    ui.within(row()).getByRole('button', { name: 'Cancel upload' }),
  );
  await ui.waitFor(() => assert.equal(calls.length, 6));
  assert.deepEqual(calls, [
    'list first',
    'list second',
    `resume ${second}`,
    'list second',
    `cancel ${second}`,
    'list second',
  ]);
  assert.deepEqual(refreshed, ['Upload resumed', 'Upload cancelled']);
});

test('a failed resume refreshes the server unless the host refused it before the agent call', async () => {
  for (const [code, refreshes] of [
    ['upload-expired', true],
    ['ambiguous', true],
    ['mutation-in-flight', false],
    ['invalid-request', false],
  ] as const) {
    const { calls, refreshed } = await mount({ code, message: `${code}.` });
    const ready = upload(STATES.indexOf('ready'), 'ready').uploadId;
    const row = ui.screen.getByText(ready).closest('li')!;
    ui.fireEvent.click(ui.within(row).getByRole('button', { name: 'Resume' }));
    await ui.screen.findByRole('alert');
    assert.deepEqual(calls, ['list first', `resume ${ready}`], code);
    assert.deepEqual(
      refreshed,
      refreshes ? ['The upload change failed; the vault was refreshed.'] : [],
      code,
    );
    ui.cleanup();
  }
});
