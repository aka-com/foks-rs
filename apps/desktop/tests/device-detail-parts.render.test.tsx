/**
 * The parts of a key's own page under Devices, rendered from their own
 * modules under screens/devices: the type word, the phrase-test chip and the
 * card operations a security key's page offers.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactNode } from 'react';

import type { YubiEnrollment } from '../src/bridge';
import type { AccountStore } from '../src/model';
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

async function render(node: ReactNode) {
  const TestShell = await loadTestShell();
  return ui.render(createElement(TestShell, { children: node }));
}

async function personalAccount(): Promise<AccountStore> {
  const { FIXTURE } = await import('../src/mock/fixture');
  const store = FIXTURE.stores.find(
    (candidate): candidate is AccountStore =>
      candidate.kind === 'account' && candidate.id === 'acct:personal',
  );
  assert.ok(store, 'the fixture holds the Personal account');
  return store;
}

test('a card enrollment is called a security key, and other kinds keep their name', async () => {
  const { kindLabel } = await import('../src/screens/devices/device-labels');
  assert.equal(kindLabel('Enrollment'), 'Security key');
  assert.equal(kindLabel('Recovery phrase'), 'Recovery phrase');
});

test('the phrase-test chip says when a phrase was never tested and when it last was', async () => {
  const { TestedChip } = await import('../src/screens/devices/device-labels');
  const { testedDate } = await import('../src/recovery-phrase-tests');
  const never = await render(
    createElement(TestedChip, { status: { state: 'never', snoozed: false } }),
  );
  assert.equal(never.container.textContent, 'Last tested: never');
  ui.cleanup();
  const at = Date.UTC(2026, 8, 16);
  const current = await render(
    createElement(TestedChip, { status: { state: 'current', testedAt: at } }),
  );
  assert.equal(current.container.textContent, `Tested ${testedDate(at)}`);
});

test('card operations act on their own enrollment and say why a step does not apply', async () => {
  const { CardOperations } =
    await import('../src/screens/devices/card-operations');
  const store = await personalAccount();
  const asked: [string, string][] = [];
  const enrollment: YubiEnrollment = {
    alias: store.account,
    state: 'complete',
    cardSerial: 1234,
  };
  const rendered = await render(
    createElement(CardOperations, {
      enrollment,
      cards: [{ serial: 1234 }],
      stopped: { stopped: false, reason: '' },
      store,
      onNavigate: () => {},
      onProvision: () => {},
      onYubiAction: (action, entry) => asked.push([action, entry.alias]),
    }),
  );
  const button = (name: string) =>
    rendered.getByRole('button', { name }) as HTMLButtonElement;
  // A complete enrollment cannot be resumed.
  assert.equal(button('Resume enrollment…').disabled, true);
  assert.equal(
    button('Resume enrollment…').title,
    'This enrollment is already complete.',
  );
  ui.fireEvent.click(button('PIN status'));
  ui.fireEvent.click(button('Sync…'));
  assert.deepEqual(asked, [
    ['pin-status', store.account],
    ['sync', store.account],
  ]);
});
