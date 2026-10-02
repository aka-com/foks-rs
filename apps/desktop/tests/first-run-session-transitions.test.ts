import assert from 'node:assert/strict';
import test from 'node:test';
import { initialFirstRun } from '../src/first-run-state';
import {
  protectionNavigationPolicy,
  selectedSigninMethod,
  setupNavigationEvent,
} from '../src/screens/first-run/session-transitions';

test('back from an uncommitted recovery phrase drops its draft but preserves passphrase input', () => {
  assert.deepEqual(
    protectionNavigationPolicy(
      { state: 'phrase', backupCommitted: false },
      'protect',
    ),
    {
      revealPhrase: false,
      clearPassphrase: false,
      discardBackup: true,
      resetAcknowledgement: true,
    },
  );
  assert.equal(
    protectionNavigationPolicy(
      { state: 'phrase', backupCommitted: true },
      'protect',
    ).discardBackup,
    false,
  );
});

test('leaving protection clears its secrets even when recovery has been committed', () => {
  const policy = protectionNavigationPolicy(
    { state: 'phrase', backupCommitted: true },
    'account',
  );
  assert.equal(policy.clearPassphrase, true);
  assert.equal(policy.discardBackup, true);
  assert.equal(policy.resetAcknowledgement, true);
  assert.equal(
    protectionNavigationPolicy(
      { state: 'protect', backupCommitted: false },
      'phrase',
    ).revealPhrase,
    true,
  );
});

test('account-method navigation uses checkpoint events rather than directly replacing state', () => {
  const saved = initialFirstRun('own', 'checked');
  assert.deepEqual(setupNavigationEvent(saved, 'existing'), {
    type: 'select-account-method',
    method: 'recover',
  });
  assert.deepEqual(setupNavigationEvent(saved, 'account'), {
    type: 'select-account-method',
    method: 'create',
  });
});

test('pending recovery cannot choose a different alias or an unavailable pairing method', () => {
  const candidate = { pairable: true, copyable: true };
  const pending = [{ kind: 'pairing-acceptance' as const, alias: 'other' }];
  assert.equal(
    selectedSigninMethod({
      candidate,
      chosen: null,
      pending,
      alias: 'selected',
    }),
    null,
  );
  assert.equal(
    selectedSigninMethod({ candidate, chosen: null, pending, alias: 'other' }),
    'pair',
  );
  assert.equal(
    selectedSigninMethod({
      candidate: null,
      chosen: 'pair',
      pending,
      alias: 'other',
    }),
    'recover',
  );
  assert.equal(
    selectedSigninMethod({
      candidate,
      chosen: 'import',
      pending,
      alias: 'other',
    }),
    'import',
  );
});
