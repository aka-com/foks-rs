import assert from 'node:assert/strict';
import test from 'node:test';
import { failureOutcome, synchronizeApplied } from '../src/operation-outcome';
const error = (code: string, ambiguous = false) => ({
  code,
  message: code,
  retryable: false,
  fatal: false,
  ambiguous,
});

test('failed hydration cannot turn a confirmed write into a rejected operation', async () => {
  let reads = 0;
  const result = await synchronizeApplied(async () => {
    reads++;
    throw error('profile-busy');
  });
  assert.equal(result.outcome, 'applied');
  assert.equal(result.synchronization, 'pending');
  assert.equal(reads, 1);
  const recovered = await synchronizeApplied(async () => ({ revision: 2 }));
  assert.deepEqual(recovered, {
    outcome: 'applied',
    synchronization: 'current',
    value: { revision: 2 },
  });
});

test('ambiguous results override apparently retryable admission codes', () => {
  assert.equal(failureOutcome(error('profile-busy')), 'unknown');
  assert.equal(failureOutcome(error('busy')), 'unknown');
  assert.equal(
    failureOutcome({
      ...error('busy'),
      details: { reason: 'admission-not-started' },
    }),
    'not-started',
  );
  assert.equal(
    failureOutcome({
      ...error('busy', true),
      details: { reason: 'admission-not-started' },
    }),
    'unknown',
  );
  assert.equal(failureOutcome(error('profile-busy', true)), 'unknown');
  assert.equal(failureOutcome(error('conflict')), 'rejected');
  assert.equal(failureOutcome(error('agent-lost')), 'unknown');
});
