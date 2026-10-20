/**
 * A copy or download whose read finds the listed item changed or replaced
 * reports it and leaves the item alone: it never opens the edit-conflict
 * sheet, whose "Refresh and review" would open the editor with no content.
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

const VAULT = 'acct:personal';

/** A read refusal as the host answers it. */
function refusal(code: string, message: string) {
  return { code, message, retryable: true, ambiguous: false, fatal: false };
}

const STALE = [
  refusal(
    'item-version-mismatch',
    'This item changed. Refresh the vault before reading it.',
  ),
  refusal(
    'item-replaced',
    'This item was replaced. Refresh the vault before reading it.',
  ),
  // The host maps the agent's own read `conflict` to item-version-mismatch;
  // the read paths do not route one into the edit-conflict sheet either.
  {
    ...refusal(
      'conflict',
      'This item changed. Refresh the vault before reading it.',
    ),
    retryable: false,
  },
];

/** The app with `path` selected in the details panel and every read refused. */
async function mount(path: string, error: unknown) {
  const { App } = await import('../src/app-root');
  const { FIXTURE } = await import('../src/mock/fixture');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const { LocationStore, INITIAL_STATE } = await import('../src/location');
  const base = mockBridge(FIXTURE);
  const refused = async (): Promise<never> => {
    throw error;
  };
  const bridge: Bridge = {
    ...base,
    copyItemValue: refused,
    copyItemField: refused,
    downloadFile: refused,
  };
  const folder = path.slice(0, path.lastIndexOf('/'));
  const store = new LocationStore({
    ...INITIAL_STATE,
    location: { kind: 'store', ref: VAULT },
    folder,
    selection: { store: VAULT, path },
    details: true,
  });
  return ui.render(createElement(App, { snapshot: FIXTURE, bridge, store }));
}

/** The details panel, once it has drawn. */
async function detailsPanel(): Promise<HTMLElement> {
  return ui.waitFor(() => {
    const panel = document.querySelector<HTMLElement>('aside.details');
    assert.ok(panel);
    return panel;
  });
}

function toasts(): string {
  return document.querySelector('.toasts')?.textContent ?? '';
}

for (const error of STALE) {
  test(`a copy refused with ${error.code} reports it without the conflict sheet`, async () => {
    const rendered = await mount('/logins/github.com', error);
    const details = await detailsPanel();
    ui.fireEvent.click(
      (
        await ui
          .within(details)
          .findAllByRole('button', { name: /Copy password/ })
      )[0],
    );
    await ui.waitFor(() =>
      assert.match(toasts(), /Refresh the vault before reading it\./),
    );
    assert.equal(rendered.queryByText('Item changed elsewhere'), null);
    assert.equal(rendered.queryByRole('dialog'), null);
  });

  test(`a download refused with ${error.code} reports it without the conflict sheet`, async () => {
    const rendered = await mount('/documents/passport-scan.pdf', error);
    const details = await detailsPanel();
    ui.fireEvent.click(
      await ui.within(details).findByRole('button', { name: /Download/ }),
    );
    await ui.waitFor(() =>
      assert.match(toasts(), /Refresh the vault before reading it\./),
    );
    assert.equal(rendered.queryByText('Item changed elsewhere'), null);
    assert.equal(rendered.queryByRole('dialog'), null);
  });
}
