import assert from 'node:assert/strict';
import test from 'node:test';

import { reconcileMutationFailure } from '../src/mutation-recovery';

test('a failed mutation refreshes once without replaying the write', async () => {
  let refreshes = 0;
  let reported: unknown;
  await reconcileMutationFailure(
    {
      code: 'invalid-request',
      message: 'write refused',
      retryable: false,
      ambiguous: false,
      fatal: false,
    },
    async () => {
      refreshes += 1;
    },
    (error) => {
      reported = error;
    },
  );
  assert.equal(refreshes, 1);
  assert.equal(reported, undefined);
});

test('agent loss leaves reconciliation to the reconnect workflow', async () => {
  let refreshes = 0;
  await reconcileMutationFailure(
    {
      code: 'agent-lost',
      message: 'disconnected',
      retryable: true,
      ambiguous: true,
      fatal: false,
    },
    async () => {
      refreshes += 1;
    },
    () => assert.fail('no refresh error should be reported'),
  );
  assert.equal(refreshes, 0);
});

test('a failed catalog reload reports its own error without masking the mutation', async () => {
  const refreshError = { code: 'catalog-failed', message: 'still unavailable' };
  let reported: unknown;
  await reconcileMutationFailure(
    {
      code: 'invalid-request',
      message: 'write refused',
      retryable: false,
      ambiguous: false,
      fatal: false,
    },
    async () => {
      throw refreshError;
    },
    (error) => {
      reported = error;
    },
  );
  assert.equal(reported, refreshError);
});
