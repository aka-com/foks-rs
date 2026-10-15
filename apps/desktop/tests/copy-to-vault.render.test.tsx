/**
 * Copy to vault in the running shell: an item copied from the details panel
 * or the row menu into another vault with the roles chosen in the sheet, the
 * original kept or moved to Trash afterwards, a Trash failure reported with
 * the copy kept, and the refresh an ambiguous copy asks for.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';

import type {
  Bridge,
  CopyItemToStoreRequest,
  RemoveItemRequest,
} from '../src/bridge';
import type { LocationStore } from '../src/location';
import { installDom } from './lib/dom-harness';
import { workflowScope } from './lib/workflow-scope';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
});

let ui: typeof import('@testing-library/react');
test.before(async () => {
  ui = await import('@testing-library/react');
});
test.afterEach(() => ui.cleanup());

const VAULT = 'acct:personal';
const ITEM = '/logins/github.com';

/**
 * The app open on the personal vault's /logins, with `selection` in the
 * details panel when given. `copies` and `trashed` record the requests that
 * reach the mock, in order, in `calls`.
 */
async function mount({
  selection,
  decorate = () => ({}),
}: {
  selection?: string;
  decorate?: (base: Bridge) => Partial<Bridge>;
} = {}) {
  const { App } = await import('../src/app-root');
  const { FIXTURE } = await import('../src/fixture');
  const { mockBridge } = await import('../src/mock-bridge');
  const { LocationStore, INITIAL_STATE } = await import('../src/location');
  const base = mockBridge(FIXTURE);
  const copies: CopyItemToStoreRequest[] = [];
  const trashed: RemoveItemRequest[] = [];
  const calls: string[] = [];
  const bridge: Bridge = {
    ...base,
    copyItemToStore: async (request) => {
      calls.push('copy');
      copies.push(request);
      return base.copyItemToStore(request);
    },
    trashItem: async (request) => {
      calls.push('trash');
      trashed.push(request);
      return base.trashItem(request);
    },
    ...decorate(base),
  };
  const store: LocationStore = new LocationStore({
    ...INITIAL_STATE,
    location: { kind: 'store', ref: VAULT },
    folder: '/logins',
    ...(selection
      ? { selection: { store: VAULT, path: selection }, details: true }
      : {}),
  });
  const rendered = ui.render(
    createElement(App, { snapshot: FIXTURE, bridge, store }),
  );
  await ui.waitFor(() => assert.ok(listRow('github.com')));
  const items = async () => (await base.listCatalog()).items;
  return { rendered, base, copies, trashed, calls, items };
}

function listRow(name: string): HTMLElement {
  const row = [
    ...document.querySelectorAll<HTMLElement>('.folder-body .row.one'),
  ].find((element) => element.querySelector('.nm')?.textContent === name);
  assert.ok(row, `${name} is listed`);
  return row;
}

function toasts(): string {
  return document.querySelector('.toasts')?.textContent ?? '';
}

function preview(dialog: HTMLElement): string {
  return dialog.querySelector('.move-preview')?.textContent ?? '';
}

async function openFromDetails(
  rendered: Awaited<ReturnType<typeof mount>>['rendered'],
): Promise<HTMLElement> {
  ui.fireEvent.click(
    await rendered.findByRole('button', { name: 'Copy to vault…' }),
  );
  return rendered.findByRole('dialog', { name: 'Copy to vault' });
}

async function chooseVault(dialog: HTMLElement, name: string): Promise<void> {
  ui.fireEvent.click(
    ui.within(dialog).getByRole('button', { name: 'Copy to vault' }),
  );
  ui.fireEvent.click(
    await ui.screen.findByRole('option', { name: new RegExp(`^${name}`) }),
  );
}

test('a login is copied into a team with the chosen roles and the original kept', async () => {
  const { rendered, copies, items } = await mount({ selection: ITEM });
  const dialog = await openFromDetails(rendered);
  await chooseVault(dialog, 'Household');
  // Household has no /logins folder, so the copy starts at the top level.
  assert.match(
    preview(dialog),
    /Copies github\.com to \/github\.com in Household\./,
  );
  ui.fireEvent.click(
    ui.within(dialog).getByRole('radio', { name: /^Admin.*can read/ }),
  );
  await ui.act(async () => {
    ui.fireEvent.click(ui.within(dialog).getByRole('button', { name: 'Copy' }));
  });
  await ui.waitFor(() =>
    assert.match(toasts(), /Copied github\.com to Household/),
  );
  assert.equal(rendered.queryByRole('dialog'), null);
  assert.equal(copies.length, 1);
  const [request] = copies;
  // The host reads the source itself: the request names it and no value.
  assert.deepEqual(Object.keys(request ?? {}).sort(), [
    'direntId',
    'path',
    'readRole',
    'storeId',
    'targetPath',
    'targetStoreId',
    'version',
    'writeRole',
  ]);
  assert.equal(request?.storeId, VAULT);
  assert.equal(request?.path, ITEM);
  assert.equal(request?.targetStoreId, 'team:household');
  assert.equal(request?.targetPath, '/github.com');
  assert.equal(request?.readRole, 'Admin');
  assert.equal(request?.writeRole, 'Admin');
  const now = await items();
  assert.ok(
    now.some(
      (item) => item.store === 'team:household' && item.path === '/github.com',
    ),
  );
  assert.ok(now.some((item) => item.store === VAULT && item.path === ITEM));
});

test('moving the original sends it to Trash after the copy is saved', async () => {
  const { rendered, calls, trashed, items } = await mount({ selection: ITEM });
  const dialog = await openFromDetails(rendered);
  await chooseVault(dialog, 'Household');
  ui.fireEvent.click(
    ui.within(dialog).getByRole('radio', {
      name: /^Move the original to Trash/,
    }),
  );
  assert.match(preview(dialog), /then moves the original to Trash/);
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(dialog).getByRole('button', { name: 'Copy and move original' }),
    );
  });
  await ui.waitFor(() =>
    assert.match(
      toasts(),
      /Moved github\.com to Household\. The original is in Trash in Personal\./,
    ),
  );
  assert.deepEqual(calls, ['copy', 'trash']);
  assert.equal(trashed[0]?.path, ITEM);
  const now = await items();
  assert.ok(!now.some((item) => item.store === VAULT && item.path === ITEM));
  assert.ok(
    now.some(
      (item) => item.store === 'team:household' && item.path === '/github.com',
    ),
  );
});

test('a Trash failure after the copy keeps the copy and reports the original as kept', async () => {
  const { rendered, items } = await mount({
    selection: ITEM,
    decorate: () => ({
      trashItem: async () => {
        throw {
          code: 'operation-failed',
          message: 'The server refused the change.',
          retryable: false,
          ambiguous: false,
          fatal: false,
        };
      },
    }),
  });
  const dialog = await openFromDetails(rendered);
  await chooseVault(dialog, 'Household');
  ui.fireEvent.click(
    ui.within(dialog).getByRole('radio', {
      name: /^Move the original to Trash/,
    }),
  );
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(dialog).getByRole('button', { name: 'Copy and move original' }),
    );
  });
  await ui.waitFor(() =>
    assert.match(
      toasts(),
      /Copied github\.com to Household\. The original is still in Personal: The server refused the change\./,
    ),
  );
  const now = await items();
  assert.ok(now.some((item) => item.store === VAULT && item.path === ITEM));
  assert.ok(
    now.some(
      (item) => item.store === 'team:household' && item.path === '/github.com',
    ),
  );
});

test('an occupied destination is refused before the copy is sent, and another server is named', async () => {
  const { rendered, copies } = await mount({ selection: ITEM });
  const dialog = await openFromDetails(rendered);
  await chooseVault(dialog, 'Household');
  ui.fireEvent.change(ui.within(dialog).getByLabelText('Name'), {
    target: { value: 'wifi' },
  });
  assert.match(
    preview(dialog),
    /Something named wifi is already at the top level of Household\./,
  );
  assert.ok(
    ui
      .within(dialog)
      .getByRole('button', { name: 'Copy' })
      .hasAttribute('disabled'),
  );
  assert.equal(copies.length, 0);

  // Personal is on the personal server; Engineering is on acme.
  await chooseVault(dialog, 'Engineering');
  assert.match(
    dialog.textContent ?? '',
    /Engineering is on .*, a different server from Personal on /,
  );
});

test('the row menu opens Copy to vault for that row', async () => {
  const { rendered } = await mount();
  ui.fireEvent.contextMenu(listRow('github.com'));
  ui.fireEvent.click(
    ui.screen.getByRole('menuitem', { name: 'Copy to vault…' }),
  );
  const dialog = await rendered.findByRole('dialog', { name: 'Copy to vault' });
  assert.match(dialog.textContent ?? '', /Copy github\.com from Personal/);
});

/**
 * The refresh scope the sheet asks for after an ambiguous copy to `vault`:
 * the item whose server is refreshed, or `undefined` for every server. The
 * sheet is rendered alone, since the mock refreshes the whole catalog either
 * way.
 */
async function ambiguousCopyScope(vault: string): Promise<string | undefined> {
  const { CopyItemSheet } = await import('../src/screens/copy-item-sheet');
  const { FIXTURE } = await import('../src/fixture');
  const { mockBridge } = await import('../src/mock-bridge');
  const { OverlayProvider } = await import('../kit/overlay-primitives');
  const item = FIXTURE.items.find(
    (entry) => entry.store === VAULT && entry.path === ITEM,
  );
  assert.ok(item);
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    copyItemToStore: async () => {
      throw {
        code: 'deadline-exceeded',
        message: 'The agent did not answer in time.',
        retryable: false,
        ambiguous: true,
        fatal: false,
      };
    },
  };
  const scopes: (string | undefined)[] = [];
  const sheet = createElement(CopyItemSheet, {
    snapshot: FIXTURE,
    bridge,
    item,
    setWorkflow: () => {},
    onApplied: async () => {},
    onMutationError: async (_failure, options) => {
      scopes.push(options?.item?.path);
    },
    accessNow: () => Math.floor(Date.now() / 1000),
  });
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  const rendered = ui.render(
    createElement(OverlayProvider, {
      backgroundRef: { current: null },
      portalRoot,
      children: await workflowScope(sheet, FIXTURE),
    }),
  );
  const dialog = await rendered.findByRole('dialog', { name: 'Copy to vault' });
  await chooseVault(dialog, vault);
  await ui.act(async () => {
    ui.fireEvent.click(ui.within(dialog).getByRole('button', { name: 'Copy' }));
  });
  await ui.waitFor(() => assert.equal(scopes.length, 1));
  assert.match(dialog.textContent ?? '', /did not answer in time/);
  return scopes[0];
}

test('an ambiguous copy to another server refreshes every server', async () => {
  // The copy held the application-wide reservation, and the source's server
  // does not hold the vault the copy may have reached.
  assert.equal(await ambiguousCopyScope('Engineering'), undefined);
});

test('an ambiguous copy within one server refreshes that server', async () => {
  assert.equal(await ambiguousCopyScope('Household'), ITEM);
});
