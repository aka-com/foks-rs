import assert from 'node:assert/strict';
import test from 'node:test';

import { reconcileMutationFailure } from '../src/mutation-recovery';

test('mutation failure triggers a single state refresh without replaying the write', async () => {
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

test('agent-lost error suppresses immediate refresh and delegates to reconnect handler', async () => {
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
    () => assert.fail('unexpected refresh error callback invocation'),
  );
  assert.equal(refreshes, 0);
});

test('catalog refresh failure passes error to report callback', async () => {
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

test('admission refusal does not launch a refresh against the running mutation', async () => {
  await reconcileMutationFailure(
    {
      code: 'mutation-in-flight',
      message: 'busy',
      retryable: false,
      ambiguous: false,
      fatal: false,
    },
    async () => {
      assert.fail('pre-dispatch refusal must not refresh');
    },
    () => assert.fail('no refresh error'),
  );
});
