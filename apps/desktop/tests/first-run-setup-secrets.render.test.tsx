/**
 * The setup secrets hook: what it holds, what `clear` drops, and what a move
 * between steps leaves behind.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { protectionNavigationPolicy } from '../src/screens/first-run/session-transitions';
import {
  holdsSetupSecret,
  useSetupSecrets,
} from '../src/screens/first-run/use-setup-secrets';
import { installDom } from './lib/dom-harness';

installDom({ timers: true, act: true });

let ui: typeof import('@testing-library/react');

test.before(async () => {
  ui = await import('@testing-library/react');
});
test.afterEach(() => ui.cleanup());

const EMPTY = {
  invite: '',
  passphrase: '',
  confirmation: '',
  recoveryPhrase: '',
  pairingPhrase: '',
  generatedRecoveryPhrase: null,
};

test('any typed or generated secret counts as held', () => {
  assert.equal(holdsSetupSecret(EMPTY), false);
  for (const field of [
    'invite',
    'passphrase',
    'confirmation',
    'recoveryPhrase',
    'pairingPhrase',
    'generatedRecoveryPhrase',
  ] as const)
    assert.equal(holdsSetupSecret({ ...EMPTY, [field]: 'x' }), true, field);
});

test('clear drops every secret and reports nothing held before the next render', () => {
  const { result } = ui.renderHook(() => useSetupSecrets());
  ui.act(() => {
    result.current.setInvite('invite');
    result.current.setPassphrase('passphrase');
    result.current.setConfirmation('passphrase');
    result.current.setRecoveryPhrase('alpha beta');
    result.current.setPairingPhrase('gamma delta');
  });
  assert.equal(result.current.held.current, true);

  let heldAfterClear: boolean | undefined;
  ui.act(() => {
    result.current.clear();
    heldAfterClear = result.current.held.current;
  });
  assert.equal(heldAfterClear, false);
  assert.equal(result.current.held.current, false);
  assert.equal(result.current.invite, '');
  assert.equal(result.current.passphrase, '');
  assert.equal(result.current.confirmation, '');
  assert.equal(result.current.recoveryPhrase, '');
  assert.equal(result.current.pairingPhrase, '');
  assert.equal(result.current.backup.generatedRecoveryPhrase, null);
});

test('a move within protection keeps the passphrase and drops typed phrases', () => {
  const { result } = ui.renderHook(() => useSetupSecrets());
  ui.act(() => {
    result.current.setInvite('invite');
    result.current.setPassphrase('passphrase');
    result.current.setConfirmation('passphrase');
    result.current.setRecoveryPhrase('alpha beta');
    result.current.setPairingPhrase('gamma delta');
  });
  ui.act(() =>
    result.current.navigate(
      protectionNavigationPolicy(
        { state: 'protect', backupCommitted: false },
        'phrase',
      ),
    ),
  );
  assert.equal(result.current.passphrase, 'passphrase');
  assert.equal(result.current.confirmation, 'passphrase');
  assert.equal(result.current.invite, '');
  assert.equal(result.current.recoveryPhrase, '');
  assert.equal(result.current.pairingPhrase, '');

  ui.act(() =>
    result.current.navigate(
      protectionNavigationPolicy(
        { state: 'phrase', backupCommitted: false },
        'account',
      ),
    ),
  );
  assert.equal(result.current.passphrase, '');
  assert.equal(result.current.confirmation, '');
  assert.equal(result.current.held.current, false);
});
