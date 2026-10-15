/**
 * The Password field of the New password sheet: Show, Generate and the
 * strength under the value.
 *
 * Generated passwords are drawn in the renderer. The tests drive the sheet
 * through the shell, take a password from the generator and check that the
 * same text reaches `createTextItem`, that the generator panel answers
 * Escape without closing the sheet, and that its settings are remembered.
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
test.afterEach(() => {
  ui.cleanup();
  window.localStorage.removeItem('passwordGenerator');
});

async function openPasswordSheet(overrides: Partial<Bridge> = {}) {
  const { App } = await import('../src/app-root');
  const { FIXTURE } = await import('../src/fixture');
  const { mockBridge } = await import('../src/mock-bridge');
  const { LocationStore, INITIAL_STATE } = await import('../src/location');
  const store = new LocationStore({
    ...INITIAL_STATE,
    location: { kind: 'store', ref: 'acct:personal' },
  });
  const rendered = ui.render(
    createElement(App, {
      snapshot: FIXTURE,
      bridge: { ...mockBridge(FIXTURE), ...overrides },
      store,
    }),
  );
  await ui.waitFor(() => assert.ok(rendered.getAllByText('Personal').length));
  ui.fireEvent.click(rendered.getByRole('button', { name: 'New' }));
  ui.fireEvent.click(
    await rendered.findByRole('menuitem', { name: 'Password' }),
  );
  const sheet = await rendered.findByRole('dialog', { name: 'New password' });
  return { rendered, sheet: ui.within(sheet) };
}

function passwordField(
  sheet: Awaited<ReturnType<typeof openPasswordSheet>>['sheet'],
): HTMLInputElement {
  return sheet.getByLabelText<HTMLInputElement>('Password');
}

test('Show reveals the typed password and a common one is warned about without blocking Create', async () => {
  const { sheet } = await openPasswordSheet();
  ui.fireEvent.change(sheet.getByLabelText('Site'), {
    target: { value: 'example.com' },
  });
  const password = passwordField(sheet);
  assert.equal(password.type, 'password');
  assert.equal(password.getAttribute('spellcheck'), 'false');

  ui.fireEvent.click(sheet.getByRole('button', { name: 'Show password' }));
  assert.equal(passwordField(sheet).type, 'text');
  ui.fireEvent.click(sheet.getByRole('button', { name: 'Hide password' }));
  assert.equal(passwordField(sheet).type, 'password');

  // The strength text is a live region that is mounted before it has text.
  const live = password.closest('.fr')?.querySelector('.pw-strength-text');
  assert.equal(live?.getAttribute('aria-live'), 'polite');
  assert.equal(live?.textContent, '');
  assert.equal(password.getAttribute('aria-describedby'), null);

  ui.fireEvent.change(password, { target: { value: 'Password1!' } });
  const warning = sheet.getByText('Weak · a common password');
  assert.equal(warning, live);
  assert.ok(warning.closest('.pw-strength.weak'));
  assert.ok(
    passwordField(sheet)
      .getAttribute('aria-describedby')
      ?.includes(warning.closest('.pw-strength')!.id),
  );
  const create = sheet.getByRole('button', { name: 'Create item' });
  assert.equal((create as HTMLButtonElement).disabled, false);

  // Losing focus masks the shown value again.
  ui.fireEvent.click(sheet.getByRole('button', { name: 'Show password' }));
  ui.fireEvent(window, new Event('blur'));
  assert.equal(passwordField(sheet).type, 'password');
  ui.fireEvent(window, new Event('focus'));
});

test('a generated password fills the field, shows its exact strength and is what Create saves', async () => {
  const created: Parameters<Bridge['createTextItem']>[0][] = [];
  const { rendered, sheet } = await openPasswordSheet({
    createTextItem: async (request) => {
      created.push(request);
      return { applied: true };
    },
  });
  ui.fireEvent.change(sheet.getByLabelText('Site'), {
    target: { value: 'example.com' },
  });

  const generate = sheet.getByRole('button', { name: 'Generate' });
  assert.equal(generate.getAttribute('aria-expanded'), 'false');
  ui.fireEvent.click(generate);
  assert.equal(generate.getAttribute('aria-expanded'), 'true');
  const panel = sheet.getByRole('group', { name: 'Generate a password' });
  assert.equal(generate.getAttribute('aria-controls'), panel.id);
  const preview = (): string =>
    ui.within(panel).getByLabelText('Generated password').textContent ?? '';
  assert.equal(preview().length, 24);
  assert.ok(ui.within(panel).getByText('Strong · 147 bits'));

  const before = preview();
  ui.fireEvent.click(
    ui.within(panel).getByRole('button', { name: 'Regenerate' }),
  );
  assert.notEqual(preview(), before);

  ui.fireEvent.click(
    ui.within(panel).getByRole('button', { name: 'Passphrase' }),
  );
  assert.equal(preview().split('-').length, 5);
  assert.ok(ui.within(panel).getByText('Good · 55 bits'));
  ui.fireEvent.change(ui.within(panel).getByLabelText('Separator'), {
    target: { value: '.' },
  });
  assert.equal(preview().split('.').length, 5);
  const chosen = preview();

  ui.fireEvent.click(
    ui.within(panel).getByRole('button', { name: 'Use password' }),
  );
  assert.equal(
    sheet.queryByRole('group', { name: 'Generate a password' }),
    null,
  );
  assert.equal(passwordField(sheet).value, chosen);
  assert.equal(document.activeElement, generate);
  assert.ok(sheet.getByText('Good · 55 bits'));

  // An edited value is no longer the draw and loses its measured strength.
  ui.fireEvent.change(passwordField(sheet), {
    target: { value: `${chosen}x` },
  });
  assert.equal(sheet.queryByText('Good · 55 bits'), null);
  ui.fireEvent.change(passwordField(sheet), { target: { value: chosen } });
  assert.ok(sheet.getByText('Good · 55 bits'));

  await ui.act(async () => {
    ui.fireEvent.click(sheet.getByRole('button', { name: 'Create item' }));
  });
  await ui.waitFor(() => assert.equal(created.length, 1));
  assert.equal(created[0].value, `user: \npassword: ${chosen}\nurl: `);
  await ui.waitFor(() =>
    assert.equal(rendered.queryByText('New password'), null),
  );
});

test('the generator answers Escape itself and remembers its settings on this device', async () => {
  const { rendered, sheet } = await openPasswordSheet();
  ui.fireEvent.click(sheet.getByRole('button', { name: 'Generate' }));
  let panel = sheet.getByRole('group', { name: 'Generate a password' });

  // The last character set cannot be turned off.
  // Each toggle's name begins with its visible text.
  for (const name of [
    'A–Z, uppercase letters',
    'a–z, lowercase letters',
    '0–9, digits',
  ])
    ui.fireEvent.click(ui.within(panel).getByRole('button', { name }));
  const status = panel.querySelector('.pw-gen-hint[role="status"]')!;
  assert.equal(status.textContent, '');
  ui.fireEvent.click(
    ui.within(panel).getByRole('button', { name: '!@#, symbols' }),
  );
  assert.equal(status.textContent, 'Keep at least one set of characters.');
  const symbols = ui.within(panel).getByRole('button', {
    name: '!@#, symbols',
  });
  assert.equal(symbols.getAttribute('aria-pressed'), 'true');
  const preview = ui.within(panel).getByLabelText('Generated password');
  assert.match(preview.textContent ?? '', /^[!@#$%^&*\-_=+?]{24}$/);
  ui.fireEvent.change(ui.within(panel).getByLabelText('Length'), {
    target: { value: '12' },
  });
  assert.equal(
    ui.within(panel).getByLabelText('Generated password').textContent?.length,
    12,
  );

  ui.fireEvent.keyDown(ui.within(panel).getByLabelText('Length'), {
    key: 'Escape',
  });
  assert.equal(
    sheet.queryByRole('group', { name: 'Generate a password' }),
    null,
  );
  assert.ok(rendered.getByRole('dialog', { name: 'New password' }));
  assert.equal(document.querySelector('[role="alertdialog"]'), null);

  const stored = JSON.parse(
    window.localStorage.getItem('passwordGenerator') ?? 'null',
  ) as Record<string, unknown>;
  assert.equal(stored.length, 12);
  assert.deepEqual(stored.sets, {
    upper: false,
    lower: false,
    digits: false,
    symbols: true,
  });

  ui.fireEvent.click(sheet.getByRole('button', { name: 'Generate' }));
  panel = sheet.getByRole('group', { name: 'Generate a password' });
  assert.equal(
    ui.within(panel).getByLabelText<HTMLInputElement>('Length').value,
    '12',
  );
  // Nothing was taken from the generator, so the field is still empty.
  assert.equal(passwordField(sheet).value, '');

  // Losing focus closes the panel, which shows its candidate in plain text.
  ui.fireEvent(window, new Event('blur'));
  assert.equal(
    sheet.queryByRole('group', { name: 'Generate a password' }),
    null,
  );
  ui.fireEvent(window, new Event('focus'));
});
