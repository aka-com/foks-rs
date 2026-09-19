/**
 * Edit and delete conflicts: the agent reports one conflict whether the item
 * was changed or removed, so the shell's refresh finds out which and says so,
 * keeping an edit whose item is gone rather than directing the user to review
 * a version that no longer exists.
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

const PATH = '/logins/github.com';

/** The app open on the personal vault's GitHub login, details showing. */
async function mount(decorate: (base: Bridge) => Partial<Bridge>) {
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
  const item = FIXTURE.items.find(
    (entry) => entry.store === 'acct:personal' && entry.path === PATH,
  );
  assert.ok(item);
  const base = mockBridge(FIXTURE);
  const bridge: Bridge = { ...base, ...decorate(base) };
  const rendered = ui.render(
    createElement(App, {
      snapshot: FIXTURE,
      bridge,
      store: new LocationStore({
        ...INITIAL_STATE,
        location: { kind: 'store', ref: 'acct:personal' },
        selection: { store: item.store, path: item.path },
        details: true,
      }),
    }),
  );
  await rendered.findByRole('button', { name: 'Edit' });
  return { rendered, item };
}

function toasts(): string {
  return document.querySelector('.toasts')?.textContent ?? '';
}

test('an edit conflict with a deleted item preserves a new-item draft', async () => {
  const { rendered } = await mount((base) => ({
    // The save meets an item another session has already deleted.
    editTextItem: async (request) => {
      await base.removeItem(request);
      return base.editTextItem(request);
    },
  }));
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Edit' }));
  const username = await rendered.findByLabelText('User name');
  ui.fireEvent.change(username, { target: { value: 'kept-draft' } });
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Save changes' }));

  // The sheet does not claim to know whether the item changed or went away.
  await rendered.findByText('Item changed elsewhere');
  assert.ok(rendered.getByText(/changed or removed while you were editing/));
  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'Refresh and review' }),
    );
  });

  // With nothing left to review, the edit is kept as a new password at the
  // same path, its fields carried over, and the toast says why.
  await rendered.findByText('New password');
  await ui.waitFor(() =>
    assert.equal(
      (rendered.getByLabelText('User name') as HTMLInputElement).value,
      'kept-draft',
    ),
  );
  assert.match(toasts(), /deleted elsewhere/);
  assert.doesNotMatch(toasts(), /Review your draft/);
});

test('an edit conflict with an existing item resumes against the refreshed version', async () => {
  const { rendered } = await mount((base) => ({
    editTextItem: async (request) => {
      // Someone else saved first; the item is still there to review.
      await base.editTextItem({ ...request, value: 'theirs' });
      return base.editTextItem(request);
    },
  }));
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Edit' }));
  const username = await rendered.findByLabelText('User name');
  ui.fireEvent.change(username, { target: { value: 'kept-draft' } });
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Save changes' }));
  await rendered.findByText('Item changed elsewhere');
  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'Refresh and review' }),
    );
  });
  await ui.waitFor(() => assert.match(toasts(), /Review your draft/));
  assert.equal(rendered.queryByText('New password'), null);
  assert.ok(document.querySelector('.details'));
});

/** The confirmation's own Delete, not the details panel's. */
function confirmDelete(): HTMLButtonElement {
  const dialog = document.querySelector('[role="alertdialog"]');
  assert.ok(dialog, 'the delete confirmation is up');
  const button = [...dialog.querySelectorAll<HTMLButtonElement>('button')].find(
    (candidate) => candidate.textContent?.trim().startsWith('Delete'),
  );
  assert.ok(button);
  return button;
}

test('delete conflicts distinguish modified and deleted items', async () => {
  let gone = false;
  const { rendered } = await mount((base) => ({
    removeItem: async (request) => {
      if (gone) {
        // Another session deleted it first.
        await base.removeItem(request);
      } else {
        // Another session changed it first.
        await base.editTextItem({ ...request, value: 'theirs' });
      }
      return base.removeItem(request);
    },
  }));
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Delete' }));
  await ui.waitFor(() => confirmDelete());
  await ui.act(async () => {
    ui.fireEvent.click(confirmDelete());
  });
  await ui.waitFor(() =>
    assert.match(toasts(), /modified by another user or session/),
  );

  gone = true;
  await ui.waitFor(() => rendered.getByRole('button', { name: 'Delete' }));
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Delete' }));
  await ui.waitFor(() => confirmDelete());
  await ui.act(async () => {
    ui.fireEvent.click(confirmDelete());
  });
  await ui.waitFor(() => assert.match(toasts(), /already deleted elsewhere/));
  assert.doesNotMatch(
    toasts().slice(toasts().indexOf('already deleted')),
    /Review the updated item/,
  );
});
