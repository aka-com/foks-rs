import assert from 'node:assert/strict';
import test from 'node:test';

import {
  classifyFirstRunFailure,
  presentFirstRunFailure,
  reconcileFirstRunFailure,
} from '../src/first-run-failure';

test('first-run uncertainty classification honors the structured flag', () => {
  const failure = classifyFirstRunFailure('server-check', {
    code: 'operation-failed',
    message: 'wording does not control recovery',
    retryable: false,
    ambiguous: true,
    fatal: false,
    details: { operation: 'check-server' },
  });
  assert.equal(failure.operation, 'server-check');
  assert.equal(failure.recovery, 'pending');
  assert.equal(failure.error.details?.operation, 'check-server');
});

test('first-run uncertainty reconciles without replaying the operation', async () => {
  const failure = classifyFirstRunFailure('account-signup', {
    code: 'response-binding',
    message: 'response could not be verified',
    retryable: false,
    ambiguous: false,
    fatal: true,
  });
  let reconciliations = 0;
  const recovered = await reconcileFirstRunFailure(failure, async () => {
    reconciliations++;
  });
  assert.equal(reconciliations, 1);
  assert.equal(recovered.recovery, 'reconciled');
  assert.match(presentFirstRunFailure(recovered).detail, /not repeated/);
});
