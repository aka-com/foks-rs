/**
 * The write overlay's sheets rendered from their own modules under
 * screens/files/sheets, without the application around them: what each
 * answers when the reader tries to leave, and what the delete sheet calls
 * its item.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactNode } from 'react';

import type { GuardVerdict, Location, LocationStore } from '../src/location';
import type { WriteWorkflow } from '../src/screens/files/write-workflow-model';
import { installDom } from './lib/dom-harness';
import { loadTestShell } from './lib/test-shell';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
  act: true,
});

let ui: typeof import('@testing-library/react');

test.before(async () => {
  ui = await import('@testing-library/react');
});
test.afterEach(() => ui.cleanup());

/** The move every test asks about: another page entirely. */
const AWAY: Location = { kind: 'files' };

async function harness() {
  const sheets = {
    ...(await import('../src/screens/files/sheets/new-folder-sheet')),
    ...(await import('../src/screens/files/sheets/exists-sheet')),
    ...(await import('../src/screens/files/sheets/delete-sheet')),
    ...(await import('../src/screens/files/sheets/conflict-sheet')),
  };
  const locations = await import('../src/location');
  const guards = await import('../src/navigation-guard');
  const { FIXTURE } = await import('../src/mock/fixture');
  const { ToastController, ToastProvider } = await import('../kit/toasts');
  const { OverlayProvider } = await import('../kit/overlay-primitives');
  const TestShell = await loadTestShell();
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  const github = FIXTURE.items.find(
    (item) => item.path === '/logins/github.com',
  );
  assert.ok(github, 'the fixture holds the GitHub login');

  /** Renders one sheet under a location store of its own. */
  const mount = (
    node: ReactNode,
  ): { store: LocationStore; rendered: ReturnType<typeof ui.render> } => {
    const store = new locations.LocationStore();
    const rendered = ui.render(
      createElement(OverlayProvider, {
        backgroundRef: { current: null },
        portalRoot,
        children: createElement(ToastProvider, {
          controller: new ToastController(),
          children: createElement(guards.NavigationGuardProvider, {
            store,
            children: createElement(TestShell, {
              snapshot: FIXTURE,
              children: node,
            }),
          }),
        }),
      }),
    );
    return { store, rendered };
  };

  return {
    sheets,
    github,
    mount,
    /** What the guards say about leaving, without moving anything. */
    verdict: (store: LocationStore): GuardVerdict =>
      store.navigationVerdict({ kind: 'navigate', location: AWAY }),
  };
}

/** Records the workflows a sheet hands back to the overlay. */
function recorder(): {
  set: (workflow: WriteWorkflow) => void;
  seen: WriteWorkflow[];
} {
  const seen: WriteWorkflow[] = [];
  return { set: (workflow) => seen.push(workflow), seen };
}

test('an empty new-folder sheet answers nothing, and a typed name asks first', async () => {
  const h = await harness();
  const workflow = recorder();
  const { store, rendered } = h.mount(
    createElement(h.sheets.NewFolderSheet, {
      workflow: { kind: 'new-folder', storeId: 'acct:personal' },
      setWorkflow: workflow.set,
    }),
  );
  assert.equal(h.verdict(store), null);
  ui.fireEvent.change(rendered.getByLabelText('Name'), {
    target: { value: 'Projects' },
  });
  const verdict = h.verdict(store);
  assert.equal(verdict?.verdict, 'prompt');
  assert.equal(
    verdict?.verdict === 'prompt' ? verdict.title : null,
    'Discard new folder?',
  );
  assert.deepEqual(workflow.seen, []);
});

test('the already-exists step asks before discarding the draft it carries', async () => {
  const h = await harness();
  const workflow = recorder();
  const { store, rendered } = h.mount(
    createElement(h.sheets.ExistsSheet, {
      workflow: {
        kind: 'exists',
        itemKind: 'Password',
        storeId: 'acct:personal',
        path: h.github.path,
      },
      setWorkflow: workflow.set,
      onOpenExisting: async () => {},
    }),
  );
  const verdict = h.verdict(store);
  assert.equal(verdict?.verdict, 'prompt');
  assert.equal(
    verdict?.verdict === 'prompt' ? verdict.body : null,
    'Your new password has not been saved.',
  );
  // Change path hands the draft back to the new-item sheet.
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Change path' }));
  assert.deepEqual(workflow.seen, [
    {
      kind: 'new',
      itemKind: 'Password',
      storeId: 'acct:personal',
      initialFolder: undefined,
      draft: undefined,
    },
  ]);
});

test('the delete sheet names an item outside Trash by its own name and says why', async () => {
  const h = await harness();
  const { rendered } = h.mount(
    createElement(h.sheets.DeleteSheet, {
      workflow: { kind: 'delete', item: h.github, outsideTrash: true },
      setWorkflow: () => {},
      onDeleteConflict: async () => {},
    }),
  );
  assert.ok(
    rendered.getByRole('heading', {
      name: 'Delete github.com permanently?',
    }),
  );
  assert.ok(rendered.getByText(h.sheets.TRASH_UNAVAILABLE_COPY));
  assert.ok(rendered.getByText(h.sheets.PERMANENT_DELETE_COPY));
});

test('Escape on the conflict sheet asks before the edit is discarded', async () => {
  const h = await harness();
  const discarded: boolean[] = [];
  const { rendered } = h.mount(
    createElement(h.sheets.ConflictSheet, {
      workflow: { kind: 'conflict', item: h.github, draft: 'edited' },
      setWorkflow: () => {},
      onRefreshConflict: async () => null,
      onDiscardConflict: () => discarded.push(true),
    }),
  );
  ui.fireEvent.keyDown(
    rendered.getByRole('dialog', { name: 'Edit conflict' }),
    { key: 'Escape' },
  );
  const confirm = rendered.getByRole('alertdialog', {
    name: 'Discard unsaved changes',
  });
  assert.deepEqual(discarded, []);
  ui.fireEvent.click(
    ui.within(confirm).getByRole('button', { name: 'Discard changes' }),
  );
  assert.deepEqual(discarded, [true]);
});
