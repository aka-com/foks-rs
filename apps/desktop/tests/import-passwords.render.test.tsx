/**
 * Import passwords in the running shell: the sheet opened from the New
 * menu lists what the host read from the file without any field value,
 * saves the logins under the chosen folder, leaves an occupied path as it
 * is, keeps the sheet open with Retry after a failure, and drops the held
 * file when it closes.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import type {
  Bridge,
  CommitPasswordImportRequest,
  PasswordImportOutcome,
} from '../src/bridge';
import type { LocationStore } from '../src/location';
import { installDom } from './lib/dom-harness';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
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

const VAULT = 'acct:personal';

/**
 * The app open on the personal vault's /logins. Commits and discards that
 * reach the bridge are recorded in order.
 */
async function mount(decorate: (base: Bridge) => Partial<Bridge> = () => ({})) {
  const { App } = (await vite.ssrLoadModule(
    '/src/app-root.tsx',
  )) as typeof import('../src/app-root');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const { LocationStore, INITIAL_STATE } = (await vite.ssrLoadModule(
    '/src/location.ts',
  )) as typeof import('../src/location');
  const base = mockBridge(FIXTURE);
  const commits: CommitPasswordImportRequest[] = [];
  const discards: string[] = [];
  const inner: Bridge = { ...base, ...decorate(base) };
  const bridge: Bridge = {
    ...inner,
    commitPasswordImport: async (request, onProgress) => {
      commits.push(request);
      return inner.commitPasswordImport(request, onProgress);
    },
    discardPasswordImport: async (importId) => {
      discards.push(importId);
      return inner.discardPasswordImport(importId);
    },
  };
  const store: LocationStore = new LocationStore({
    ...INITIAL_STATE,
    location: { kind: 'store', ref: VAULT },
    folder: '/logins',
  });
  const rendered = ui.render(
    createElement(App, { snapshot: FIXTURE, bridge, store }),
  );
  await ui.waitFor(() =>
    assert.ok(document.querySelector('.folder-body .row')),
  );
  const items = async () => (await base.listCatalog()).items;
  return { rendered, commits, discards, items };
}

function toasts(): string {
  return document.querySelector('.toasts')?.textContent ?? '';
}

function rowTexts(dialog: HTMLElement, label: string): string[] {
  return [
    ...ui
      .within(dialog)
      .getByRole('list', { name: label })
      .querySelectorAll('li'),
  ].map((row) => row.textContent ?? '');
}

/** Opens the sheet from the New menu and reads the mock's file. */
async function openPreview(
  rendered: Awaited<ReturnType<typeof mount>>['rendered'],
): Promise<HTMLElement> {
  ui.fireEvent.click(rendered.getByRole('button', { name: 'New' }));
  ui.fireEvent.click(
    await ui.screen.findByRole('menuitem', { name: 'Import passwords…' }),
  );
  const dialog = await rendered.findByRole('dialog', {
    name: 'Import passwords',
  });
  assert.match(dialog.textContent ?? '', /plain text/);
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(dialog).getByRole('button', { name: 'Choose CSV file…' }),
    );
  });
  return rendered.findByRole('dialog', { name: 'Import 3 logins' });
}

test('the preview lists names and folders, and an import saves what is missing', async () => {
  const { rendered, commits, discards, items } = await mount();
  const dialog = await openPreview(rendered);
  // The current folder is the destination; a login keeps its own folder.
  assert.deepEqual(
    rowTexts(dialog, 'Logins').map((text) => text.trim()),
    ['/logins/github.com', '/logins/Work/Mail', '/logins/Bank'],
  );
  assert.deepEqual(rowTexts(dialog, 'Rows left out'), [
    'Row 5: Wi-Fi codesA note entry, not a login.',
  ]);
  assert.match(
    dialog.textContent ?? '',
    /Columns not imported: favorite, login_totp\./,
  );
  assert.doesNotMatch(dialog.textContent ?? '', /imported-|satoshi/);

  ui.fireEvent.change(ui.within(dialog).getByLabelText('Folder'), {
    target: { value: '/.trash/x' },
  });
  assert.match(dialog.textContent ?? '', /cannot be imported into Trash/);
  assert.ok(
    ui
      .within(dialog)
      .getByRole('button', { name: 'Import 3 logins' })
      .hasAttribute('disabled'),
  );
  ui.fireEvent.change(ui.within(dialog).getByLabelText('Folder'), {
    target: { value: '/logins' },
  });
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(dialog).getByRole('button', { name: 'Import 3 logins' }),
    );
  });
  // github.com is already in /logins, so it is left as it is.
  await ui.waitFor(() =>
    assert.match(toasts(), /Imported 2 logins to Personal; 1 already there/),
  );
  assert.equal(rendered.queryByRole('dialog'), null);
  assert.equal(commits.length, 1);
  // The request names the held file and the destination, and no value.
  assert.deepEqual(Object.keys(commits[0] ?? {}).sort(), [
    'entries',
    'folder',
    'importId',
    'storeId',
  ]);
  assert.deepEqual(commits[0]?.entries, [0, 1, 2]);
  assert.equal(commits[0]?.storeId, VAULT);
  assert.equal(commits[0]?.folder, '/logins');
  const now = await items();
  for (const path of ['/logins/Work/Mail', '/logins/Bank'])
    assert.ok(
      now.some((item) => item.store === VAULT && item.path === path),
      path,
    );
  // The sheet drops the held file when it closes.
  assert.deepEqual(discards, [commits[0]?.importId]);
});

test('a failure keeps the sheet open, and Retry sends only what was not saved', async () => {
  let attempts = 0;
  const { rendered, commits } = await mount((base) => ({
    commitPasswordImport: async (request, onProgress) => {
      attempts++;
      if (attempts > 1) return base.commitPasswordImport(request, onProgress);
      const outcomes: PasswordImportOutcome[] = [
        { index: 0, status: 'exists', message: 'Already there.' },
        { index: 1, status: 'failed', message: 'The server is unavailable.' },
        {
          index: 2,
          status: 'not-started',
          message: 'Not started: an earlier login failed.',
        },
      ];
      outcomes.forEach((outcome) => onProgress?.(outcome));
      return outcomes;
    },
  }));
  const dialog = await openPreview(rendered);
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(dialog).getByRole('button', { name: 'Import 3 logins' }),
    );
  });
  const result = await rendered.findByRole('dialog', {
    name: 'Imported 0 of 3 logins',
  });
  assert.match(
    ui.within(result).getByRole('status').textContent ?? '',
    /1 already there · 2 not saved/,
  );
  assert.match(
    rowTexts(result, 'Logins')[1] ?? '',
    /The server is unavailable\./,
  );
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(result).getByRole('button', { name: 'Retry' }),
    );
  });
  await ui.waitFor(() =>
    assert.match(toasts(), /Imported 2 logins to Personal; 1 already there/),
  );
  assert.deepEqual(
    commits.map((request) => request.entries),
    [
      [0, 1, 2],
      [1, 2],
    ],
  );
});

test('cancelling the preview drops the held file without saving anything', async () => {
  const { rendered, commits, discards } = await mount();
  const dialog = await openPreview(rendered);
  ui.fireEvent.click(ui.within(dialog).getByRole('button', { name: 'Cancel' }));
  await ui.waitFor(() => assert.equal(rendered.queryByRole('dialog'), null));
  assert.equal(commits.length, 0);
  assert.equal(discards.length, 1);
});
