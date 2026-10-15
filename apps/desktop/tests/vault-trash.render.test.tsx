/**
 * Vault Trash in the running shell: Delete moves an item or a folder into
 * Trash with Undo, the Trash view restores entries (as a copy when the
 * original path is taken, and from a hashed entry's `.origin` file), and
 * only entries in Trash are deleted permanently, except in a team whose
 * Trash this account cannot set up. Empty Trash and the optional automatic
 * emptying remove only the entries this account can change.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';

import type { RenderResult } from '@testing-library/react';
import type { Bridge } from '../src/bridge';
import type { LocationStore } from '../src/location';
import { installDom } from './lib/dom-harness';

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

const PATH = '/logins/github.com';

/**
 * The app open on one item with its details showing: by default the
 * personal vault's GitHub login. `setup` changes the mock's vault first.
 */
async function mount({
  decorate = () => ({}),
  setup,
  vault = 'acct:personal',
  path = PATH,
}: {
  decorate?: (base: Bridge) => Partial<Bridge>;
  setup?: (base: Bridge) => Promise<void>;
  vault?: string;
  path?: string;
} = {}) {
  const { App } = await import('../src/app-root');
  const { FIXTURE } = await import('../src/mock/fixture');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const { LocationStore, INITIAL_STATE } = await import('../src/location');
  const base = mockBridge(FIXTURE);
  await setup?.(base);
  const bridge: Bridge = { ...base, ...decorate(base) };
  const store: LocationStore = new LocationStore({
    ...INITIAL_STATE,
    location: { kind: 'store', ref: vault },
    selection: { store: vault, path },
    details: true,
  });
  const rendered = ui.render(
    createElement(App, { snapshot: FIXTURE, bridge, store }),
  );
  await rendered.findByRole('button', { name: 'Move to Trash' });
  if (setup) {
    // The shell starts from the fixture; read what `setup` changed.
    const [refresh] = rendered.getAllByRole('button', {
      name: /— refresh vaults, teams/,
    });
    await ui.act(async () => {
      ui.fireEvent.click(refresh);
    });
    await ui.waitFor(() =>
      assert.equal(
        rendered.queryAllByRole('button', { name: /— refreshing vaults/ })
          .length,
        0,
      ),
    );
  }
  const paths = async () =>
    (await base.listCatalog()).items
      .filter((item) => item.store === vault)
      .map((item) => item.path);
  return { rendered, store, base, paths };
}

function toasts(): string {
  return document.querySelector('.toasts')?.textContent ?? '';
}

async function trashGithub(rendered: RenderResult) {
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Move to Trash' }));
  });
  await ui.waitFor(() => assert.match(toasts(), /Moved github\.com to Trash/));
}

function trashRow(name: string): HTMLElement {
  const row = [
    ...document.querySelectorAll<HTMLElement>('.trash-view .row'),
  ].find((element) => element.querySelector('.nm')?.textContent === name);
  assert.ok(row, `${name} is listed in Trash`);
  return row;
}

test('Move to Trash removes the item at once and Undo puts it back', async () => {
  const { rendered, paths } = await mount();
  await trashGithub(rendered);
  assert.equal(document.querySelector('[role="alertdialog"]'), null);
  // Trash, the owners' Trash folder, and the entry.
  const moved = (await paths()).filter(
    (path) => path === '/.trash' || path.startsWith('/.trash/'),
  );
  assert.equal(moved.length, 3);
  assert.ok(
    moved.some((path) =>
      /^\/\.trash\/owner\/\d{8}T\d{6}~%2Flogins%2Fgithub\.com$/.test(path),
    ),
  );
  assert.ok(!(await paths()).includes(PATH));
  // Trash is not an ordinary folder in the tree.
  assert.equal(
    document.querySelector('.tpane [data-folder-path="/.trash"]'),
    null,
  );

  const undo = [
    ...document.querySelectorAll<HTMLButtonElement>('.toast-action'),
  ].find((button) => button.textContent === 'Undo');
  assert.ok(undo);
  await ui.act(async () => {
    ui.fireEvent.click(undo);
  });
  await ui.waitFor(() => assert.match(toasts(), /Restored github\.com/));
  assert.ok((await paths()).includes(PATH));
});

test('⌘Z undoes the last move to Trash while its toast is offered', async () => {
  const { rendered, paths } = await mount();
  await trashGithub(rendered);
  await ui.act(async () => {
    ui.fireEvent.keyDown(document.body, { key: 'z', metaKey: true });
  });
  await ui.waitFor(async () => assert.ok((await paths()).includes(PATH)));
  // The Undo is spent: a second ⌘Z changes nothing.
  const before = await paths();
  await ui.act(async () => {
    ui.fireEvent.keyDown(document.body, { key: 'z', metaKey: true });
  });
  assert.deepEqual(await paths(), before);
});

test('the Undo toast closes when ⌘Z stops undoing, even while it is hovered', async (t) => {
  // jsdom reports a hidden window, which would hold the toast as well.
  Object.defineProperty(document, 'hidden', {
    configurable: true,
    get: () => false,
  });
  t.after(() => {
    delete (document as { hidden?: boolean }).hidden;
  });
  const { rendered, paths } = await mount();
  await trashGithub(rendered);
  const toast = [...document.querySelectorAll<HTMLElement>('.toast')].find(
    (element) => /Moved github\.com to Trash/.test(element.textContent ?? ''),
  );
  assert.ok(toast);
  ui.fireEvent.mouseEnter(toast);
  // The pointer stays on the toast past the eight seconds of Undo.
  const later = Date.now() + 9_000;
  t.mock.method(Date, 'now', () => later);
  ui.fireEvent.mouseLeave(toast);
  ui.fireEvent.mouseEnter(toast);
  await ui.act(async () => {
    ui.fireEvent.keyDown(document.body, { key: 'z', metaKey: true });
  });
  await ui.waitFor(() =>
    assert.equal(
      [...document.querySelectorAll('.toast-action')].some(
        (button) => button.textContent === 'Undo',
      ),
      false,
    ),
  );
  assert.ok(!(await paths()).includes(PATH));
});

test('the Trash view restores, offers a copy for an occupied path, and deletes permanently', async () => {
  const { rendered, store, base, paths } = await mount();
  await trashGithub(rendered);
  // Another github.com is saved where the old one was.
  await base.createTextItem({
    storeId: 'acct:personal',
    path: PATH,
    value: 'user: new',
  });
  await ui.act(async () => store.setFolder('/.trash'));
  await ui.waitFor(() => assert.ok(trashRow('github.com')));
  assert.match(
    trashRow('github.com').textContent ?? '',
    /\/logins/,
    'the original location is shown',
  );
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(trashRow('github.com')).getByRole('button', {
        name: 'Restore github.com',
      }),
    );
  });
  const conflict = await rendered.findByRole('dialog');
  assert.match(conflict.textContent ?? '', /\/logins\/github\.com/);
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(conflict).getByRole('button', {
        name: 'Restore as github.com (restored)',
      }),
    );
  });
  await ui.waitFor(async () =>
    assert.ok((await paths()).includes(`${PATH} (restored)`)),
  );
  assert.ok((await paths()).includes(PATH));

  // Delete permanently names what it cannot promise.
  const pdf = 'passport-scan.pdf';
  await ui.act(async () => store.setFolder(''));
  await ui.act(async () =>
    store.select({ store: 'acct:personal', path: `/documents/${pdf}` }),
  );
  await ui.act(async () => {
    ui.fireEvent.click(
      await rendered.findByRole('button', { name: 'Move to Trash' }),
    );
  });
  await ui.waitFor(() =>
    assert.match(toasts(), /Moved passport-scan\.pdf to Trash/),
  );
  await ui.act(async () => store.setFolder('/.trash'));
  await ui.waitFor(() => assert.ok(trashRow(pdf)));
  ui.fireEvent.click(
    ui.within(trashRow(pdf)).getByRole('button', {
      name: `Delete ${pdf} permanently`,
    }),
  );
  const sheet = await rendered.findByRole('alertdialog');
  assert.match(
    sheet.textContent ?? '',
    /Removes it for everyone\. Earlier encrypted versions may remain on the server\./,
  );
  assert.doesNotMatch(sheet.textContent ?? '', /cannot be undone/);
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(sheet).getByRole('button', { name: 'Delete permanently' }),
    );
  });
  await ui.waitFor(async () =>
    assert.ok(
      !(await paths()).some((path) => path.includes('%2Fpassport-scan.pdf')),
    ),
  );
});

test('the Trash list is a grid with one tab stop that the arrow keys walk', async () => {
  const { rendered, store } = await mount();
  await trashGithub(rendered);
  const pdf = 'passport-scan.pdf';
  await ui.act(async () =>
    store.select({ store: 'acct:personal', path: `/documents/${pdf}` }),
  );
  await ui.act(async () => {
    ui.fireEvent.click(
      await rendered.findByRole('button', { name: 'Move to Trash' }),
    );
  });
  await ui.waitFor(() =>
    assert.match(toasts(), /Moved passport-scan\.pdf to Trash/),
  );
  await ui.act(async () => store.setFolder('/.trash'));
  await ui.waitFor(() => assert.ok(trashRow(pdf)));
  const grid = rendered.getByRole('grid', { name: 'Trash · Personal' });
  const rows = [...grid.querySelectorAll<HTMLElement>('.row')];
  assert.equal(rows.length, 2);
  assert.equal(grid.getAttribute('aria-rowcount'), '3');
  assert.deepEqual(
    ui
      .within(grid)
      .getAllByRole('columnheader')
      .map((cell) => cell.textContent),
    ['Name', 'Original location', 'Deleted', 'Size', 'Actions'],
  );
  assert.deepEqual(
    rows.map((row) => [
      row.getAttribute('role'),
      row.getAttribute('aria-rowindex'),
      row.querySelectorAll('[role="gridcell"]').length,
    ]),
    [
      ['row', '2', 5],
      ['row', '3', 5],
    ],
  );
  const stops = () => [...grid.querySelectorAll<HTMLElement>('[tabindex="0"]')];
  assert.deepEqual(stops(), [rows[0]]);
  for (const button of grid.querySelectorAll('.trash-acts button'))
    assert.equal(button.getAttribute('tabindex'), '-1');
  rows[0].focus();
  ui.fireEvent.keyDown(rows[0], { key: 'ArrowDown' });
  assert.equal(document.activeElement, rows[1]);
  assert.deepEqual(stops(), [rows[1]]);
  ui.fireEvent.keyDown(rows[1], { key: 'Home' });
  assert.equal(document.activeElement, rows[0]);
  // Right walks the row's Restore and Delete permanently; Left walks back.
  const actions = [...rows[0].querySelectorAll<HTMLElement>('button')];
  ui.fireEvent.keyDown(rows[0], { key: 'ArrowRight' });
  assert.equal(document.activeElement, actions[0]);
  ui.fireEvent.keyDown(actions[0], { key: 'ArrowRight' });
  assert.equal(document.activeElement, actions[1]);
  ui.fireEvent.keyDown(actions[1], { key: 'ArrowLeft' });
  ui.fireEvent.keyDown(actions[0], { key: 'ArrowLeft' });
  assert.equal(document.activeElement, rows[0]);
  ui.fireEvent.keyDown(rows[0], { key: 'Enter' });
  assert.equal(rows[0].getAttribute('aria-selected'), 'true');
  assert.equal(rows[1].getAttribute('aria-selected'), 'false');
  // Delete permanently takes Delete or ⌘Backspace with no other modifier,
  // as the registry lists it.
  ui.fireEvent.keyDown(rows[0], { key: 'Delete', shiftKey: true });
  ui.fireEvent.keyDown(rows[0], { key: 'Backspace' });
  assert.equal(rendered.queryByRole('alertdialog'), null);
  ui.fireEvent.keyDown(rows[0], { key: 'Backspace', ctrlKey: true });
  const sheet = await rendered.findByRole('alertdialog');
  assert.ok(
    ui.within(sheet).getByRole('button', { name: 'Delete permanently' }),
  );
});

test('a folder moves to Trash whole, restores whole, and Empty Trash removes every entry', async () => {
  const { rendered, store, paths } = await mount();
  const folder = document.querySelector<HTMLElement>(
    '.tpane [data-folder-store="acct:personal"][data-folder-path="/ssh"] .fselect',
  );
  assert.ok(folder);
  ui.fireEvent.contextMenu(folder);
  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('menuitem', { name: 'Move folder to Trash' }),
    );
  });
  await ui.waitFor(() => assert.match(toasts(), /Moved ssh to Trash/));
  assert.ok(
    (await paths()).some((path) =>
      /^\/\.trash\/owner\/\d{8}T\d{6}~%2Fssh\/id_ed25519$/.test(path),
    ),
  );
  await ui.act(async () => store.setFolder('/.trash'));
  await ui.waitFor(() =>
    assert.match(trashRow('ssh').textContent ?? '', /1 item/),
  );
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(trashRow('ssh')).getByRole('button', { name: 'Restore ssh' }),
    );
  });
  await ui.waitFor(async () =>
    assert.ok((await paths()).includes('/ssh/id_ed25519')),
  );

  await trashGithubFromTrashView(rendered, store);
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Empty Trash…' }));
  const sheet = await rendered.findByRole('alertdialog');
  assert.match(sheet.textContent ?? '', /Permanently delete 1 item\?/);
  assert.equal(
    document.activeElement,
    ui.within(sheet).getByRole('button', { name: 'Cancel' }),
  );
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(sheet).getByRole('button', { name: 'Empty Trash' }),
    );
  });
  await ui.waitFor(async () =>
    assert.deepEqual(
      (await paths()).filter((path) => path.startsWith('/.trash/owner/')),
      [],
    ),
  );
  await ui.waitFor(() => assert.ok(rendered.getByText('Trash is empty')));
});

async function trashGithubFromTrashView(
  rendered: RenderResult,
  store: LocationStore,
) {
  await ui.act(async () => store.setFolder(''));
  await ui.act(async () =>
    store.select({ store: 'acct:personal', path: PATH }),
  );
  await ui.act(async () => {
    ui.fireEvent.click(
      await rendered.findByRole('button', { name: 'Move to Trash' }),
    );
  });
  await ui.waitFor(() => assert.match(toasts(), /Moved github\.com to Trash/));
  await ui.act(async () => store.setFolder('/.trash'));
  await ui.waitFor(() => assert.ok(trashRow('github.com')));
}

/** Moves an item to Trash through the mock, as another session would. */
async function trashThrough(base: Bridge, vault: string, path: string) {
  const item = (await base.listCatalog()).items.find(
    (candidate) => candidate.store === vault && candidate.path === path,
  );
  assert.ok(item, `${path} is in ${vault}`);
  const moved = await base.trashItem({
    storeId: vault,
    path,
    version: item.version,
    direntId: item.direntId,
  });
  // The host's catalog is read again before the next selection.
  await base.listCatalog();
  return moved.path;
}

test('⌘Z replaces the Undo toast, so its button cannot restore a second time', async () => {
  let restores = 0;
  const { rendered, paths } = await mount({
    decorate: (base) => ({
      restoreTrashItem: (request) => {
        restores += 1;
        return base.restoreTrashItem(request);
      },
    }),
  });
  await trashGithub(rendered);
  await ui.act(async () => {
    ui.fireEvent.keyDown(document.body, { key: 'z', metaKey: true });
  });
  await ui.waitFor(async () => assert.ok((await paths()).includes(PATH)));
  assert.equal(
    [...document.querySelectorAll('.toast-action')].some(
      (button) => button.textContent === 'Undo',
    ),
    false,
  );
  assert.equal(restores, 1);
});

test('a name already in Trash is reported with the host message, not as a changed item', async () => {
  const { rendered } = await mount({
    decorate: () => ({
      trashItem: async () => {
        throw {
          code: 'trash-name-taken',
          message:
            'An entry with the same name is already in Trash. Try again in a second.',
          retryable: false,
          ambiguous: false,
          fatal: false,
        };
      },
    }),
  });
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Move to Trash' }));
  });
  await ui.waitFor(() =>
    assert.match(document.body.textContent ?? '', /Try again in a second/),
  );
  assert.doesNotMatch(toasts(), /changed by another user/);
});

test('a hashed entry shows the name and location its .origin file records, and restores there', async () => {
  const long = `/notes/${'n'.repeat(240)}`;
  const { rendered, store, paths } = await mount({
    setup: async (base) => {
      await base.createTextItem({
        storeId: 'acct:personal',
        path: long,
        value: 'kept',
      });
      await base.listCatalog();
      await trashThrough(base, 'acct:personal', long);
    },
  });
  const trashed = (await paths()).filter((path) =>
    path.startsWith('/.trash/owner/'),
  );
  assert.equal(trashed.length, 2);
  assert.ok(trashed.some((path) => /~#[0-9a-f]{32}\.origin$/.test(path)));
  await ui.act(async () => store.setFolder('/.trash'));
  const row = await ui.waitFor(() => trashRow('n'.repeat(240)));
  assert.match(row.textContent ?? '', /\/notes/);
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(row).getByRole('button', {
        name: `Restore ${'n'.repeat(240)}`,
      }),
    );
  });
  await ui.waitFor(async () => assert.ok((await paths()).includes(long)));
  assert.deepEqual(
    (await paths()).filter((path) => path.startsWith('/.trash/owner/')),
    [],
  );
  assert.ok(rendered.getByText('Trash is empty'));
});

test('Empty Trash removes several entries in one host call and keeps those this account cannot change', async () => {
  const calls: number[] = [];
  const { rendered, store, paths } = await mount({
    vault: 'team:eng',
    path: '/onboarding/README.md',
    setup: async (base) => {
      for (const path of [
        '/deploy/production-token',
        '/deploy/staging-token',
        '/release/bundle.tar',
      ])
        await trashThrough(base, 'team:eng', path);
    },
    decorate: (base) => ({
      emptyTrash: (request) => {
        calls.push(request.entries.length);
        return base.emptyTrash(request);
      },
    }),
  });
  await ui.act(async () => store.setFolder('/.trash'));
  await ui.waitFor(() => assert.ok(trashRow('staging-token')));
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Empty Trash…' }));
  const sheet = await rendered.findByRole('alertdialog');
  // This Admin cannot change the Owner-written production token.
  assert.match(sheet.textContent ?? '', /Permanently delete 2 items\?/);
  assert.match(sheet.textContent ?? '', /1 item stays in Trash/);
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(sheet).getByRole('button', { name: 'Empty Trash' }),
    );
  });
  await ui.waitFor(() => assert.match(toasts(), /Emptied Trash/));
  assert.deepEqual(calls, [2]);
  const left = (await paths()).filter((path) =>
    /^\/\.trash\/[^/]+\/[^/]+$/.test(path),
  );
  assert.equal(left.length, 1);
  assert.match(
    left[0],
    /^\/\.trash\/admin\/\d{8}T\d{6}~%2Fdeploy%2Fproduction-token$/,
  );
});

test('a member who cannot set up a team Trash is offered permanent deletion instead', async () => {
  const { rendered, store, paths } = await mount({
    vault: 'team:eng',
    path: '/deploy/staging-token',
  });
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Move to Trash' }));
  });
  const sheet = await rendered.findByRole('alertdialog');
  assert.match(sheet.textContent ?? '', /Trash is not set up in this team yet/);
  assert.match(sheet.textContent ?? '', /Removes it for everyone/);
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(sheet).getByRole('button', { name: 'Delete permanently' }),
    );
  });
  await ui.waitFor(async () =>
    assert.ok(!(await paths()).includes('/deploy/staging-token')),
  );
  assert.ok(!(await paths()).includes('/.trash'));
  await ui.act(async () => store.setFolder('/.trash'));
  await ui.waitFor(() =>
    assert.match(
      document.querySelector('.trash-view')?.textContent ?? '',
      /An owner sets it up by moving any item to Trash/,
    ),
  );
});

const TRASH_REFUSED = {
  code: 'trash-unavailable',
  message:
    'Trash could not be created at the top level of this vault. In a team, only an owner can create it.',
  retryable: false,
  ambiguous: false,
  fatal: false,
};

test('the host refusing to create a team Trash leads a non-owner to permanent deletion', async () => {
  // The catalog shows a `/.trash` that is gone by the time the host moves
  // the item, and only an owner can create it again.
  const { rendered } = await mount({
    vault: 'team:eng',
    path: '/deploy/staging-token',
    setup: async (base) => {
      await base.createFolder({
        storeId: 'team:eng',
        path: '/.trash',
        readRole: 'Member:0',
        writeRole: 'Owner',
      });
      await base.listCatalog();
    },
    decorate: () => ({
      trashItem: async () => {
        throw TRASH_REFUSED;
      },
    }),
  });
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Move to Trash' }));
  });
  const sheet = await rendered.findByRole('alertdialog');
  assert.match(sheet.textContent ?? '', /Trash is not set up in this team yet/);
});

test('an owner whose team Trash cannot be created sees the error, not permanent deletion', async () => {
  const { rendered } = await mount({
    vault: 'team:household',
    path: '/wifi/guest-password',
    decorate: () => ({
      trashItem: async () => {
        throw TRASH_REFUSED;
      },
    }),
  });
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Move to Trash' }));
  });
  await ui.waitFor(() =>
    assert.match(document.body.textContent ?? '', /Trash could not be created/),
  );
  assert.equal(rendered.queryByRole('alertdialog'), null);
});

/** Automatic emptying starts once the catalog has been quiet this long. */
const AUTO_EMPTY_WAIT_MS = 3_000 + 2_000;

/** An expired entry in the personal Trash, filed there by `setup`. */
async function fileExpiredEntry(base: Bridge) {
  const item = (await base.listCatalog()).items.find(
    (candidate) =>
      candidate.store === 'acct:personal' &&
      candidate.path === '/ssh/id_ed25519',
  );
  assert.ok(item);
  await base.moveItem({
    storeId: 'acct:personal',
    path: item.path,
    version: item.version,
    direntId: item.direntId,
    destination: '/.trash/owner/20200101T000000~%2Fssh%2Fid_ed25519',
  });
  await base.listCatalog();
}

test('automatic emptying, when chosen on this device, removes entries past the chosen age', async () => {
  window.localStorage.setItem('trashAutoEmptyDays', '30');
  try {
    const { rendered, paths } = await mount({
      setup: async (base) => {
        const recent = await trashThrough(
          base,
          'acct:personal',
          '/documents/passport-scan.pdf',
        );
        assert.ok(recent);
        await fileExpiredEntry(base);
      },
    });
    await ui.waitFor(
      () => assert.match(toasts(), /Deleted 1 entry permanently from Trash/),
      { timeout: AUTO_EMPTY_WAIT_MS },
    );
    const left = (await paths()).filter((path) =>
      path.startsWith('/.trash/owner/'),
    );
    assert.equal(left.length, 1);
    assert.match(left[0], /%2Fdocuments%2Fpassport-scan\.pdf$/);
    assert.ok(rendered);
  } finally {
    window.localStorage.removeItem('trashAutoEmptyDays');
  }
});

test('Move to Trash during automatic emptying runs after it instead of being ignored', async () => {
  window.localStorage.setItem('trashAutoEmptyDays', '30');
  let release = () => {};
  let started = false;
  const order: string[] = [];
  try {
    const { rendered, paths } = await mount({
      setup: fileExpiredEntry,
      decorate: (base) => ({
        emptyTrash: async (request) => {
          started = true;
          await new Promise<void>((resolve) => {
            release = resolve;
          });
          order.push('empty');
          return base.emptyTrash(request);
        },
        trashItem: (request) => {
          order.push('trash');
          return base.trashItem(request);
        },
      }),
    });
    await ui.waitFor(() => assert.ok(started), { timeout: AUTO_EMPTY_WAIT_MS });
    await ui.act(async () => {
      ui.fireEvent.click(
        rendered.getByRole('button', { name: 'Move to Trash' }),
      );
    });
    assert.deepEqual(order, []);
    await ui.act(async () => release());
    await ui.waitFor(() =>
      assert.match(toasts(), /Moved github\.com to Trash/),
    );
    assert.deepEqual(order, ['empty', 'trash']);
    assert.ok(!(await paths()).includes(PATH));
  } finally {
    release();
    window.localStorage.removeItem('trashAutoEmptyDays');
  }
});
