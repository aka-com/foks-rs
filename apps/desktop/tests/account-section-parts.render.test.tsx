/**
 * Parts of Settings › Account rendered from their own modules under
 * features/settings/account, without the section around them: the secondary
 * account workflows and the notices drawn when an account is missing.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactNode } from 'react';

import type { AccountStore } from '../src/model';
import type { AccountSheet } from '../src/features/settings/account/account-panel';
import { installDom } from './lib/dom-harness';
import { loadTestShell } from './lib/test-shell';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div>',
  timers: true,
  act: true,
});

let ui: typeof import('@testing-library/react');

test.before(async () => {
  ui = await import('@testing-library/react');
});
test.afterEach(() => ui.cleanup());

async function harness() {
  const { FIXTURE } = await import('../src/mock/fixture');
  const TestShell = await loadTestShell();
  const accounts = FIXTURE.stores.filter(
    (store): store is AccountStore => store.kind === 'account',
  );
  assert.ok(accounts.length >= 2, 'the fixture holds two accounts');
  return {
    snapshot: FIXTURE,
    accounts,
    render: (node: ReactNode) =>
      ui.render(createElement(TestShell, { children: node })),
  };
}

test('Import from FOKS CLI asks the section for the profile sheet', async () => {
  const { AccountActions } =
    await import('../src/features/settings/account/account-actions');
  const h = await harness();
  const opened: AccountSheet[] = [];
  const rendered = h.render(
    createElement(AccountActions, {
      snapshot: h.snapshot,
      store: h.accounts[0],
      onSheet: (sheet) => opened.push(sheet),
    }),
  );
  ui.fireEvent.click(
    rendered.getByRole('button', { name: 'Import from FOKS CLI' }),
  );
  assert.deepEqual(opened, ['go-profile']);
});

test('a missing account offers Refresh and every account still on this device', async () => {
  const { UnavailableAccount } =
    await import('../src/features/settings/account/unavailable-account');
  const h = await harness();
  const chosen: string[] = [];
  let refreshed = 0;
  const rendered = h.render(
    createElement(UnavailableAccount, {
      stores: h.accounts,
      snapshot: h.snapshot,
      onSelect: (store) => chosen.push(store.id),
      onRefresh: () => {
        refreshed += 1;
      },
    }),
  );
  const buttons = rendered.getAllByRole('button');
  assert.equal(buttons.length, 1 + h.accounts.length);
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Refresh' }));
  ui.fireEvent.click(buttons[2]);
  assert.equal(refreshed, 1);
  assert.deepEqual(chosen, [h.accounts[1].id]);
  // With accounts left to choose, the notice does not claim there are none.
  assert.equal(rendered.queryByText(/This device has no active account/), null);
});

test('with no account left, the missing-account notice says how to get one', async () => {
  const { UnavailableAccount } =
    await import('../src/features/settings/account/unavailable-account');
  const h = await harness();
  const rendered = h.render(
    createElement(UnavailableAccount, {
      stores: [],
      snapshot: h.snapshot,
      onSelect: () => {},
      onRefresh: () => {},
    }),
  );
  assert.ok(rendered.getByText(/This device has no active account/));
});
