/**
 * Testing a written recovery phrase from Devices: the row's last-tested chip,
 * the 17-field sheet with per-token checks and completion, each answer the
 * agent can give, including revoked keys, a match local storage refuses, the
 * overdue reminder, and the recall step that replaces the
 * "I have written down these words" checkbox when a phrase is created there.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';

import type { Bridge } from '../src/bridge';
import type { AgentSnapshot } from '../src/model';
import { installDom } from './lib/dom-harness';

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
test.beforeEach(() => window.localStorage.clear());
test.afterEach(() => {
  ui.cleanup();
  window.localStorage.clear();
});

/** The mock's written copy of the personal account's recovery phrase. */
const PHRASE =
  'orbit 4071 velvet 128 cactus 16 mirror 2077 harbor 64 pistol 7310 thumb 512 copper 3306 ivory';
const BACKUP_ID = `10${'4'.repeat(64)}`;
const DAY = 24 * 60 * 60 * 1000;

async function renderDevices(
  decorate: (bridge: Bridge) => Bridge = (bridge) => bridge,
  scene = 'devices',
) {
  const { FIXTURE } = await import('../src/fixture');
  const { DevicesScreen } = await import('../src/screens/devices-screen');
  const { mockBridge } = await import('../src/mock-bridge');
  const { ToastController, ToastProvider } = await import('../kit/toasts');
  const { OverlayProvider } = await import('../kit/overlay-primitives');
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  const snapshot: AgentSnapshot = FIXTURE;
  const bridge = decorate(mockBridge(snapshot));
  const errors: unknown[] = [];
  const rendered = ui.render(
    createElement(OverlayProvider, {
      backgroundRef: { current: null },
      portalRoot,
      children: createElement(ToastProvider, {
        controller: new ToastController(),
        children: createElement(DevicesScreen, {
          snapshot,
          bridge,
          location: { kind: 'devices', store: 'acct:personal' },
          scene,
          onNavigate: () => {},
          onRefresh: async () => {},
          onRefreshSnapshot: async () => snapshot,
          onError: (error: unknown) => errors.push(error),
          onMutationError: async (error: unknown) => {
            throw error;
          },
        }),
      }),
    }),
  );
  await ui.act(async () => {
    await Promise.resolve();
  });
  return Object.assign(rendered, { errors });
}

async function paperRow(rendered: Awaited<ReturnType<typeof renderDevices>>) {
  const name = await rendered.findByText('paper-backup');
  const row = name.closest('.devrow');
  assert.ok(row instanceof HTMLElement);
  return row;
}

async function openTest(rendered: Awaited<ReturnType<typeof renderDevices>>) {
  const row = await paperRow(rendered);
  await ui.act(async () => {
    ui.fireEvent.click(ui.within(row).getByRole('button', { name: 'Test…' }));
  });
  const dialog = rendered.getByRole('dialog', { name: 'Test recovery phrase' });
  const fields = [
    ...dialog.querySelectorAll<HTMLInputElement>('.phrase-test-grid input'),
  ];
  assert.equal(fields.length, 17);
  return { dialog, fields };
}

function paste(field: HTMLInputElement, text: string): void {
  ui.fireEvent.paste(field, {
    clipboardData: { getData: () => text },
  });
}

async function submit(dialog: HTMLElement): Promise<void> {
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(dialog).getByRole('button', { name: 'Test phrase' }),
    );
    await Promise.resolve();
  });
}

test('an untested recovery phrase says so on its row and in the test sheet', async () => {
  const rendered = await renderDevices();
  const row = await paperRow(rendered);
  assert.ok(ui.within(row).getByText('Last tested: never'));
  const band = rendered.getByRole('group', {
    name: 'Recovery phrase not tested',
  });
  assert.ok(ui.within(band).getByText(/No written copy of paper-backup/));
  const { dialog, fields } = await openTest(rendered);
  const [first, second] = fields;
  assert.ok(first && second);
  assert.equal(first.getAttribute('aria-label'), 'Position 1, word 1 of 9');
  assert.equal(second.getAttribute('aria-label'), 'Position 2, number 1 of 8');
  assert.ok(ui.within(dialog).getByText(/It is not sent to the server/));
  const button = ui.within(dialog).getByRole('button', { name: 'Test phrase' });
  assert.equal((button as HTMLButtonElement).disabled, true);

  // Each field reports what its position can detect.
  ui.fireEvent.change(first, { target: { value: 'orbx' } });
  assert.ok(ui.within(dialog).getByText('Unknown word'));
  ui.fireEvent.change(first, { target: { value: 'orbit' } });
  ui.fireEvent.change(second, { target: { value: 'velvet' } });
  assert.ok(ui.within(dialog).getByText('Expected a number'));
  assert.ok(ui.within(dialog).getByText(/usually means a token was skipped/));
  ui.fireEvent.change(second, { target: { value: '9000' } });
  assert.ok(ui.within(dialog).getByText('Numbers go from 0 to 8191'));
  assert.ok(ui.within(dialog).getByText(/1 field to fix/));
});

test('word fields complete from the BIP-39 list', async () => {
  const rendered = await renderDevices();
  const { dialog, fields } = await openTest(rendered);
  const [first, second] = fields;
  assert.ok(first && second);
  first.focus();
  ui.fireEvent.change(first, { target: { value: 'cac' } });
  const list = ui.within(dialog).getByRole('listbox', {
    name: 'Matching words',
  });
  assert.deepEqual(
    ui
      .within(list)
      .getAllByRole('option')
      .map((option) => option.textContent),
    ['cactus'],
  );
  assert.equal(first.getAttribute('aria-expanded'), 'true');
  ui.fireEvent.keyDown(first, { key: 'Enter' });
  assert.equal(first.value, 'cactus');
  assert.equal(document.activeElement, second);
  // Backspace in an empty field moves back.
  ui.fireEvent.keyDown(second, { key: 'Backspace' });
  assert.equal(document.activeElement, first);
  // Shift+Tab closes the list without accepting a word.
  ui.fireEvent.change(first, { target: { value: 'ab' } });
  assert.ok(ui.within(dialog).getByRole('listbox'));
  ui.fireEvent.keyDown(first, { key: 'Tab', shiftKey: true });
  assert.equal(first.value, 'ab');
  assert.equal(ui.within(dialog).queryByRole('listbox'), null);
});

test('typed tokens are masked while the window is inactive', async () => {
  const rendered = await renderDevices();
  const { fields } = await openTest(rendered);
  const first = fields[0];
  assert.ok(first);
  ui.fireEvent.change(first, { target: { value: 'orbit' } });
  await ui.act(async () => {
    ui.fireEvent(window, new Event('blur'));
  });
  assert.ok(fields.every((field) => field.type === 'password'));
  // Focusing a field shows the tokens again.
  await ui.act(async () => {
    ui.fireEvent(window, new Event('focus'));
    fields[1]?.focus();
  });
  assert.ok(fields.every((field) => field.type === 'text'));
  assert.equal(first.value, 'orbit');
});

test('a matching copy records the test on this device and clears the fields', async () => {
  const calls: [string, string][] = [];
  const rendered = await renderDevices((bridge) => ({
    ...bridge,
    verifyRecoveryPhrase: async (store, phrase) => {
      calls.push([store, phrase]);
      return bridge.verifyRecoveryPhrase(store, phrase);
    },
  }));
  const { dialog, fields } = await openTest(rendered);
  const first = fields[0];
  assert.ok(first);
  // A pasted phrase keeps its boundaries, one token per field.
  paste(first, `${PHRASE.replaceAll(' ', '\n')}\n`);
  assert.deepEqual(
    fields.map((field) => field.value),
    PHRASE.split(' '),
  );
  assert.ok(ui.within(dialog).getByText('17 of 17'));
  await submit(dialog);
  assert.deepEqual(calls, [['acct:personal', PHRASE]]);
  assert.ok(
    await ui
      .within(dialog)
      .findByText('Matches recovery phrase paper-backup (orbit 4071)'),
  );
  assert.ok(ui.within(dialog).getByText('Today, on this device'));
  assert.equal(dialog.querySelector('.phrase-test-grid'), null);
  const stored = JSON.parse(
    window.localStorage.getItem('foks.recoveryPhraseTests') ?? '{}',
  ) as Record<string, { testedAt?: number }>;
  assert.equal(typeof stored[BACKUP_ID]?.testedAt, 'number');
  assert.doesNotMatch(JSON.stringify(stored), /orbit|velvet/);

  await ui.act(async () => {
    ui.fireEvent.click(ui.within(dialog).getByRole('button', { name: 'Done' }));
  });
  assert.equal(rendered.queryByRole('dialog'), null);
  const row = await paperRow(rendered);
  assert.ok(ui.within(row).getByText(/^Tested /));
  assert.equal(ui.within(row).queryByText('Last tested: never'), null);
  assert.equal(
    rendered.queryByRole('group', { name: 'Recovery phrase not tested' }),
    null,
  );
});

test('a match that local storage refuses is not reported as recorded', async () => {
  const rendered = await renderDevices();
  const { dialog, fields } = await openTest(rendered);
  const first = fields[0];
  assert.ok(first);
  paste(first, PHRASE);
  const prototype = Object.getPrototypeOf(window.localStorage) as Storage;
  const descriptor = Object.getOwnPropertyDescriptor(prototype, 'setItem');
  assert.ok(descriptor);
  prototype.setItem = () => {
    throw new Error('Storage is full');
  };
  try {
    await submit(dialog);
  } finally {
    Object.defineProperty(prototype, 'setItem', descriptor);
  }
  assert.ok(
    await ui
      .within(dialog)
      .findByText('Matches recovery phrase paper-backup (orbit 4071)'),
  );
  assert.equal(ui.within(dialog).queryByText('Today, on this device'), null);
  assert.ok(ui.within(dialog).getByText('Not recorded'));
  assert.ok(
    ui.within(dialog).getByText(/could not store the time of this test/),
  );
  assert.equal(window.localStorage.getItem('foks.recoveryPhraseTests'), null);
});

/** The mock bridge, answering every check for a revoked backup key. */
function revokedBridge(bridge: Bridge): Bridge {
  return {
    ...bridge,
    verifyRecoveryPhrase: async (store, phrase) => ({
      ...(await bridge.verifyRecoveryPhrase(store, phrase)),
      revoked: true,
      backupAlias: null,
    }),
  };
}

test('a revoked match is reported, not recorded, and can be followed by another test', async () => {
  const rendered = await renderDevices(revokedBridge);
  const { dialog, fields } = await openTest(rendered);
  const first = fields[0];
  assert.ok(first);
  paste(first, PHRASE);
  await submit(dialog);
  assert.ok(
    await ui
      .within(dialog)
      .findByText('This is recovery phrase orbit 4071, which was revoked'),
  );
  assert.ok(ui.within(dialog).getByText('Revoked'));
  assert.ok(ui.within(dialog).getByText(/Destroy this copy/));
  assert.equal(ui.within(dialog).queryByRole('button', { name: 'Done' }), null);
  assert.ok(ui.within(dialog).getByRole('button', { name: 'Close' }));
  assert.equal(window.localStorage.getItem('foks.recoveryPhraseTests'), null);

  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(dialog).getByRole('button', { name: 'Test another phrase' }),
    );
    await new Promise((resolve) => queueMicrotask(() => resolve(undefined)));
  });
  const again = [
    ...dialog.querySelectorAll<HTMLInputElement>('.phrase-test-grid input'),
  ];
  assert.equal(again.length, 17);
  assert.ok(again.every((field) => field.value === ''));
  assert.equal(document.activeElement, again[0]);
});

test('a near miss of a revoked phrase says a corrected copy cannot recover the account', async () => {
  const rendered = await renderDevices(revokedBridge);
  const { dialog, fields } = await openTest(rendered);
  const tokens = PHRASE.split(' ');
  tokens[4] = 'cage';
  paste(fields[0], tokens.join(' '));
  await submit(dialog);
  assert.ok(
    await ui
      .within(dialog)
      .findByText('This phrase starts like orbit 4071 but does not match it'),
  );
  assert.ok(
    ui
      .within(dialog)
      .getByText(/even a corrected copy cannot recover this account/),
  );
  assert.ok(ui.within(dialog).getByRole('button', { name: 'Edit phrase' }));
  assert.equal(window.localStorage.getItem('foks.recoveryPhraseTests'), null);
});

test('a near miss names the phrase and keeps the fields for editing', async () => {
  const rendered = await renderDevices();
  const { dialog, fields } = await openTest(rendered);
  const tokens = PHRASE.split(' ');
  tokens[4] = 'cage';
  paste(fields[0], tokens.join(' '));
  await submit(dialog);
  assert.ok(
    await ui
      .within(dialog)
      .findByText(
        'This phrase starts like paper-backup (orbit 4071) but does not match it',
      ),
  );
  assert.ok(ui.within(dialog).getByText(/positions 3\s+to 17 is different/));
  assert.equal(window.localStorage.getItem('foks.recoveryPhraseTests'), null);

  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(dialog).getByRole('button', { name: 'Edit phrase' }),
    );
    await new Promise((resolve) => queueMicrotask(() => resolve(undefined)));
  });
  const again = [
    ...dialog.querySelectorAll<HTMLInputElement>('.phrase-test-grid input'),
  ];
  assert.deepEqual(
    again.map((field) => field.value),
    tokens,
  );
  // Positions 1 and 2 already agree, so editing starts at position 3.
  assert.equal(document.activeElement, again[2]);
});

test('an unrelated copy and an agent refusal are reported in the sheet', async () => {
  let refuse = false;
  const rendered = await renderDevices((bridge) => ({
    ...bridge,
    verifyRecoveryPhrase: async (store, phrase) => {
      if (refuse)
        throw {
          code: 'invalid-request',
          message:
            'Word 2 of the recovery phrase is not a recognized word. Check its spelling.',
          retryable: false,
          ambiguous: false,
          fatal: false,
        };
      return bridge.verifyRecoveryPhrase(store, phrase);
    },
  }));
  const { dialog, fields } = await openTest(rendered);
  const tokens = PHRASE.split(' ');
  tokens[0] = 'cage';
  paste(fields[0], tokens.join(' '));
  await submit(dialog);
  assert.ok(
    await ui
      .within(dialog)
      .findByText(
        'This phrase does not match any recovery phrase on this account',
      ),
  );
  // The result names the copy by its first word and number.
  assert.equal(ui.within(dialog).getAllByText('cage 4071').length, 2);

  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(dialog).getByRole('button', { name: 'Edit phrase' }),
    );
  });
  refuse = true;
  await submit(dialog);
  assert.ok(
    await ui
      .within(dialog)
      .findByText(
        'Word 2 of the recovery phrase is not a recognized word. Check its spelling.',
      ),
  );
  assert.deepEqual(rendered.errors, []);
});

test('an overdue phrase shows a reminder that can be snoozed for 30 days', async () => {
  window.localStorage.setItem(
    'foks.recoveryPhraseTests',
    JSON.stringify({
      [BACKUP_ID]: { testedAt: Date.now() - 200 * DAY },
    }),
  );
  const rendered = await renderDevices();
  const band = await rendered.findByRole('group', {
    name: 'Recovery phrase due for a test',
  });
  assert.ok(ui.within(band).getByText(/more than 180 days ago/));
  const row = await paperRow(rendered);
  assert.ok(ui.within(row).getByText(/^Last tested /));
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(band).getByRole('button', { name: 'Remind me later' }),
    );
  });
  assert.equal(
    rendered.queryByRole('group', { name: 'Recovery phrase due for a test' }),
    null,
  );
  const stored = JSON.parse(
    window.localStorage.getItem('foks.recoveryPhraseTests') ?? '{}',
  ) as Record<string, { snoozedUntil?: number }>;
  const until = stored[BACKUP_ID]?.snoozedUntil ?? 0;
  assert.ok(Math.abs(until - (Date.now() + 30 * DAY)) < 60_000);
});

test('creating a phrase on Devices asks for three tokens before it is saved', async () => {
  const committed: string[] = [];
  const rendered = await renderDevices(
    (bridge) => ({
      ...bridge,
      commitOwnerBackup: async (_profile, _account, _alias, phrase) => {
        committed.push(phrase);
        return { applied: true };
      },
    }),
    'settings-phrase',
  );
  const dialog = await ui.waitFor(() => rendered.getByRole('dialog'));
  assert.equal(
    ui.within(dialog).queryByRole('checkbox', {
      name: 'I have written down these words',
    }),
    null,
  );
  ui.fireEvent.click(ui.within(dialog).getByRole('button', { name: 'Next' }));
  assert.ok(ui.within(dialog).getByText('Check your copy'));
  assert.equal(dialog.querySelector('.words'), null);
  const save = ui
    .within(dialog)
    .getByRole('button', { name: 'Save recovery phrase' });
  assert.equal((save as HTMLButtonElement).disabled, true);
  const tokens = PHRASE.split(' ');
  for (const input of dialog.querySelectorAll<HTMLInputElement>(
    '.recall-check input',
  )) {
    const position = Number(
      /position (\d+)/.exec(input.labels?.[0]?.textContent ?? '')?.[1],
    );
    ui.fireEvent.change(input, { target: { value: tokens[position - 1] } });
  }
  assert.ok(ui.within(dialog).getByText('All three match your phrase.'));
  assert.equal((save as HTMLButtonElement).disabled, false);
  await ui.act(async () => {
    ui.fireEvent.click(save);
    await Promise.resolve();
  });
  await ui.waitFor(() => assert.deepEqual(committed, [PHRASE]));
  // Three tokens are not a test of the whole copy: nothing is recorded, so
  // the row still asks for a full test.
  assert.equal(window.localStorage.getItem('foks.recoveryPhraseTests'), null);
});
