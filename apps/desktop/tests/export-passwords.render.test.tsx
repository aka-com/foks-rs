/**
 * Export passwords in the running shell: opened from the vault's folder
 * menu, it states that the file is plain text and is sent only once the
 * vault's name is typed; the host writes the file, so the request names the
 * store alone.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';

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

const VAULT = 'acct:personal';

async function mount(decorate: (base: Bridge) => Partial<Bridge> = () => ({})) {
  const { App } = await import('../src/app-root');
  const { FIXTURE } = await import('../src/mock/fixture');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const { LocationStore, INITIAL_STATE } = await import('../src/location');
  const base = mockBridge(FIXTURE);
  const inner: Bridge = { ...base, ...decorate(base) };
  const exports: string[] = [];
  const bridge: Bridge = {
    ...inner,
    exportLogins: async (storeId) => {
      exports.push(storeId);
      return inner.exportLogins(storeId);
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
  return { rendered, exports };
}

function toasts(): string {
  return document.querySelector('.toasts')?.textContent ?? '';
}

async function openExport(
  rendered: Awaited<ReturnType<typeof mount>>['rendered'],
): Promise<HTMLElement> {
  const root = document.querySelector<HTMLElement>(
    `[data-folder-store="${VAULT}"][data-folder-path="/"]`,
  );
  assert.ok(root);
  ui.fireEvent.contextMenu(root);
  ui.fireEvent.click(
    await ui.screen.findByRole('menuitem', { name: 'Export passwords…' }),
  );
  return rendered.findByRole('dialog', { name: 'Export passwords' });
}

test('the export waits for the vault name and sends only the store', async () => {
  const { rendered, exports } = await mount();
  const dialog = await openExport(rendered);
  assert.match(dialog.textContent ?? '', /plain text/);
  // Values are written as stored, so the sheet warns against spreadsheets.
  assert.match(dialog.textContent ?? '', /not a\s+spreadsheet app/);
  const button = ui.within(dialog).getByRole('button', { name: 'Export…' });
  assert.ok(button.hasAttribute('disabled'));
  const field = ui.within(dialog).getByLabelText('Type Personal to confirm');
  ui.fireEvent.change(field, { target: { value: 'personal' } });
  assert.ok(button.hasAttribute('disabled'));
  ui.fireEvent.change(field, { target: { value: 'Personal' } });
  await ui.act(async () => {
    ui.fireEvent.click(button);
  });
  await ui.waitFor(() =>
    assert.match(toasts(), /Exported \d+ passwords? from Personal/),
  );
  assert.equal(rendered.queryByRole('dialog'), null);
  assert.deepEqual(exports, [VAULT]);
});

test('a cancelled save dialog keeps the sheet open, and a failure is shown', async () => {
  let attempts = 0;
  const { rendered } = await mount(() => ({
    exportLogins: async () => {
      attempts++;
      if (attempts === 1) return { saved: false, exported: 0, skipped: 0 };
      throw {
        code: 'export-write',
        message: 'Could not write the export: disk full',
        retryable: true,
        ambiguous: false,
        fatal: false,
      };
    },
  }));
  const dialog = await openExport(rendered);
  ui.fireEvent.change(
    ui.within(dialog).getByLabelText('Type Personal to confirm'),
    { target: { value: 'Personal' } },
  );
  const button = ui.within(dialog).getByRole('button', { name: 'Export…' });
  await ui.act(async () => {
    ui.fireEvent.click(button);
  });
  assert.ok(rendered.getByRole('dialog', { name: 'Export passwords' }));
  await ui.act(async () => {
    ui.fireEvent.click(button);
  });
  assert.match(
    ui.within(dialog).getByRole('alert').textContent ?? '',
    /disk full/,
  );
});
