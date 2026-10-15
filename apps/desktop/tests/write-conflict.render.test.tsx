/**
 * Edit and Trash conflicts: the agent reports one conflict whether the item
 * was changed or removed, so the shell's refresh finds out which and says so,
 * keeping an edit whose item is gone rather than directing the user to review
 * a version that no longer exists.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';

import type { Bridge } from '../src/bridge';
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

/** The app open on the personal vault's GitHub login, details showing. */
async function mount(decorate: (base: Bridge) => Partial<Bridge>) {
  const { App } = await import('../src/app-root');
  const { FIXTURE } = await import('../src/fixture');
  const { mockBridge } = await import('../src/mock-bridge');
  const { LocationStore, INITIAL_STATE } = await import('../src/location');
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
    // The save meets an item another session has already moved to Trash.
    editTextItem: async (request) => {
      await base.trashItem({
        ...request,
        direntId: (await base.listCatalog()).items.find(
          (item) =>
            item.store === request.storeId && item.path === request.path,
        )!.direntId,
      });
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

test('a deleted login resumes as a Document when the Password sheet cannot hold every line', async () => {
  const { conflictDraftWorkflow } =
    await import('../src/screens/write-workflows');
  const { FIXTURE } = await import('../src/fixture');
  const item = FIXTURE.items.find(
    (entry) => entry.store === 'acct:personal' && entry.path === PATH,
  );
  assert.ok(item);

  /** The kind and draft of the sheet a deleted item's edit resumes in. */
  const resume = (draft: string) => {
    const workflow = conflictDraftWorkflow(item, draft);
    assert.ok(workflow?.kind === 'new');
    return { itemKind: workflow.itemKind, draft: workflow.draft };
  };

  const login = resume('user: kept\npassword:  padded \nurl: \n');
  assert.equal(login.itemKind, 'Password');
  assert.equal(login.draft?.password, ' padded ');

  for (const draft of [
    'ssid: Cabin\npassword: x',
    'user: a\npassword: x\nrotate this every quarter',
    'password: first\npassword: second',
  ]) {
    const resumed = resume(draft);
    assert.equal(resumed.itemKind, 'Document', draft);
    assert.equal(resumed.draft?.value, draft);
  }
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

test('Move to Trash conflicts distinguish modified and removed items', async () => {
  let gone = false;
  const { rendered } = await mount((base) => ({
    trashItem: async (request) => {
      if (gone) {
        // Another session moved it to Trash first.
        await base.trashItem({
          ...request,
          direntId: (await base.listCatalog()).items.find(
            (item) =>
              item.store === request.storeId && item.path === request.path,
          )!.direntId,
        });
      } else {
        // Another session changed it first.
        await base.editTextItem({ ...request, value: 'theirs' });
      }
      return base.trashItem(request);
    },
  }));
  // Moving to Trash can be undone, so it asks for no confirmation.
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Move to Trash' }));
  });
  assert.equal(document.querySelector('[role="alertdialog"]'), null);
  await ui.waitFor(() =>
    assert.match(toasts(), /changed by another user or session/),
  );

  gone = true;
  await ui.waitFor(() =>
    rendered.getByRole('button', { name: 'Move to Trash' }),
  );
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Move to Trash' }));
  });
  await ui.waitFor(() =>
    assert.match(toasts(), /already moved or deleted elsewhere/),
  );
});
