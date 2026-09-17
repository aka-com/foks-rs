/**
 * Render tests for dragging a file onto a vault's content area.
 *
 * The runtime reports drops for the whole window, so these drive the bridge's
 * drop events directly and assert on what the vault view does with them.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import type { Bridge } from '../src/bridge';
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

/** The drop events the native window would emit, under the test's control. */
interface DropDriver {
  hover(hovering: boolean): Promise<void>;
  drop(paths: string[]): Promise<void>;
}

async function mount(
  location: { kind: 'store'; ref: string } | { kind: 'all' },
  overrides: Partial<Bridge> = {},
  browsing?: Partial<import('../src/location').LocationState>,
  reshape?: (
    snapshot: import('../src/model').AgentSnapshot,
  ) => import('../src/model').AgentSnapshot,
) {
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
  const snapshot = reshape ? reshape(FIXTURE) : FIXTURE;

  const hovers = new Set<(event: { hovering: boolean }) => void>();
  const paths = new Set<(dropped: string[]) => void>();
  const bridge: Bridge = {
    ...mockBridge(snapshot),
    onDropHover: async (listener) => {
      hovers.add(listener);
      return () => hovers.delete(listener);
    },
    onDropPaths: async (listener) => {
      paths.add(listener);
      return () => paths.delete(listener);
    },
    ...overrides,
  };
  const rendered = ui.render(
    createElement(App, {
      snapshot,
      bridge,
      store: new LocationStore({ ...INITIAL_STATE, location, ...browsing }),
    }),
  );
  const driver: DropDriver = {
    hover: async (hovering) => {
      await ui.act(async () => {
        for (const listener of [...hovers]) listener({ hovering });
      });
    },
    drop: async (dropped) => {
      await ui.act(async () => {
        for (const listener of [...paths]) listener(dropped);
      });
    },
  };
  return { rendered, driver, snapshot };
}

test('a drag over a vault names the destination, and the drop uploads there', async () => {
  // The store's own root is the default folder, so the uploaded item lands
  // where the browser is already looking.
  const { rendered, driver } = await mount(
    { kind: 'store', ref: 'acct:personal' },
    {},
  );
  await ui.waitFor(() => assert.ok(rendered.getAllByText('Personal').length));

  await driver.hover(true);
  assert.ok(rendered.getByText('Drop to upload to Personal'));
  assert.ok(rendered.getByText(/Saved at the top level/));

  await driver.hover(false);
  assert.equal(rendered.queryByText('Drop to upload to Personal'), null);

  await driver.drop(['/Users/ray/report.pdf']);
  await ui.waitFor(() => {
    assert.ok(rendered.getByText('Uploaded report.pdf to Personal'));
    assert.ok(rendered.getByText('report.pdf'));
  });
  // The veil clears once the upload settles.
  assert.equal(rendered.queryByText(/Uploading/), null);
});

test('a drop into a group uses the roles the new-item sheet defaults to', async () => {
  const uploads: unknown[] = [];
  const { rendered, driver } = await mount(
    { kind: 'store', ref: 'team:eng' },
    {
      importDroppedFile: async (request) => {
        uploads.push(request);
        return { applied: true };
      },
    },
  );
  await ui.waitFor(() =>
    assert.ok(rendered.getAllByText('Engineering').length),
  );

  await driver.hover(true);
  assert.ok(rendered.getByText('Drop to upload to Engineering'));
  await driver.drop(['/Users/ray/keys/backup.tar']);
  await ui.waitFor(() => assert.equal(uploads.length, 1));
  assert.deepEqual(uploads[0], {
    storeId: 'team:eng',
    path: '/backup.tar',
    sourcePath: '/Users/ray/keys/backup.tar',
    readRole: 'Member:0',
    writeRole: 'Admin',
  });
});

test('dropping several files at once uploads none of them', async () => {
  const uploads: unknown[] = [];
  const { rendered, driver } = await mount(
    { kind: 'store', ref: 'acct:personal' },
    {
      importDroppedFile: async (request) => {
        uploads.push(request);
        return { applied: true };
      },
    },
  );
  await ui.waitFor(() => assert.ok(rendered.getAllByText('Personal').length));

  await driver.drop(['/Users/ray/one.pdf', '/Users/ray/two.pdf']);
  await ui.waitFor(() =>
    assert.ok(rendered.getByText('Drop one file at a time to upload it.')),
  );
  assert.equal(uploads.length, 0);
});

test('dropping a file onto an existing path displays the conflict resolution step instead of overwriting', async () => {
  // The drop lands in the selected folder, where the fixture already keeps
  // a file of this name.
  const { rendered, driver, snapshot } = await mount(
    { kind: 'store', ref: 'acct:personal' },
    {},
    { folder: '/documents' },
  );
  const taken = snapshot.items.find(
    (item) =>
      item.store === 'acct:personal' && item.path.startsWith('/documents/'),
  );
  assert.ok(taken, 'the fixture has a file in the personal vault documents');
  const name = taken.path.split('/').at(-1);
  assert.ok(name);
  await ui.waitFor(() => assert.ok(rendered.getAllByText('Personal').length));

  await driver.drop([`/Users/ray/${name}`]);
  await ui.waitFor(() =>
    assert.ok(rendered.getByText(`An item already exists at ${taken.path}`)),
  );
});

test('the all-items view has no single destination, so it takes no drop', async () => {
  const uploads: unknown[] = [];
  const { rendered, driver } = await mount(
    { kind: 'all' },
    {
      importDroppedFile: async (request) => {
        uploads.push(request);
        return { applied: true };
      },
    },
  );
  await ui.waitFor(() => assert.ok(rendered.getAllByText('All items').length));

  await driver.hover(true);
  assert.equal(rendered.queryByText(/Drop to upload/), null);
  await driver.drop(['/Users/ray/report.pdf']);
  assert.equal(uploads.length, 0);
});

test('a drop while browsing a folder saves into the folder on screen', async () => {
  const uploads: unknown[] = [];
  const { rendered, driver } = await mount(
    { kind: 'store', ref: 'acct:personal' },
    {
      importDroppedFile: async (request) => {
        uploads.push(request);
        return { applied: true };
      },
    },
    { folder: '/ssh' },
  );
  await ui.waitFor(() => assert.ok(rendered.getAllByText('Personal').length));

  await driver.hover(true);
  assert.ok(rendered.getByText(/Saved in \/ssh/));
  await driver.drop(['/Users/ray/id_rsa.pub']);
  await ui.waitFor(() => assert.equal(uploads.length, 1));
  assert.deepEqual(uploads[0], {
    storeId: 'acct:personal',
    path: '/ssh/id_rsa.pub',
    sourcePath: '/Users/ray/id_rsa.pub',
  });
});

test('the new-item sheet keeps the drop while it is open', async () => {
  const uploads: unknown[] = [];
  const { rendered, driver } = await mount(
    { kind: 'store', ref: 'acct:personal' },
    {
      importDroppedFile: async (request) => {
        uploads.push(request);
        return { applied: true };
      },
    },
  );
  await ui.waitFor(() => assert.ok(rendered.getAllByText('Personal').length));
  ui.fireEvent.click(rendered.getByRole('button', { name: 'New' }));
  ui.fireEvent.click(
    await rendered.findByRole('menuitem', { name: 'Document' }),
  );
  await ui.waitFor(() => assert.ok(rendered.getByText('New document')));

  await driver.hover(true);
  assert.equal(rendered.queryByText(/Drop to upload/), null);
  await driver.drop(['/Users/ray/report.pdf']);
  // The sheet stages the file for its own Create step; nothing is uploaded yet.
  assert.equal(uploads.length, 0);
  assert.ok(rendered.getByText('report.pdf'));
});

test('replacing a file in the details panel takes the drop from the vault', async () => {
  const uploads: unknown[] = [];
  const { rendered, driver, snapshot } = await mount(
    { kind: 'store', ref: 'acct:personal' },
    {
      importDroppedFile: async (request) => {
        uploads.push(request);
        return { applied: true };
      },
    },
    // Search flattens the browser, so the file is one click away from the
    // details panel without opening its folder first.
    { query: 'id_ed25519' },
  );
  const file = snapshot.items.find(
    (item) => item.store === 'acct:personal' && item.path === '/ssh/id_ed25519',
  );
  assert.ok(file);
  await ui.waitFor(() => assert.ok(rendered.getAllByText('Personal').length));
  ui.fireEvent.click(rendered.getByText('id_ed25519'));
  ui.fireEvent.click(await rendered.findByRole('button', { name: 'Edit' }));
  await ui.waitFor(() => assert.ok(rendered.getByText('Replace')));

  await driver.hover(true);
  // The panel's own zone answers the drag; the vault behind it stays quiet.
  assert.ok(rendered.getByText('Release to replace file'));
  assert.equal(rendered.queryByText(/Drop to upload/), null);
  await driver.drop(['/Users/ray/id_ed25519']);
  await ui.waitFor(() =>
    assert.ok(rendered.getByText('id_ed25519', { selector: '.mono' })),
  );
  assert.equal(uploads.length, 0);
});

test('read-only vault rejects drag-and-drop file upload and displays error explanation', async () => {
  const uploads: unknown[] = [];
  const { rendered, driver } = await mount(
    { kind: 'store', ref: 'team:household' },
    {
      importDroppedFile: async (request) => {
        uploads.push(request);
        return { applied: true };
      },
    },
    undefined,
    (snapshot) => ({
      ...snapshot,
      parties: snapshot.parties.map((party) =>
        party.store === 'team:household' && party.label === 'you'
          ? { ...party, locally_manageable: false }
          : party,
      ),
    }),
  );
  await ui.waitFor(() => assert.ok(rendered.getAllByText('Household').length));

  await driver.hover(true);
  assert.ok(rendered.getByText('Cannot upload to Household'));
  assert.ok(
    rendered.getByText('You do not have write permissions for this team.'),
  );
  await driver.drop(['/Users/ray/report.pdf']);
  assert.equal(uploads.length, 0);
});
