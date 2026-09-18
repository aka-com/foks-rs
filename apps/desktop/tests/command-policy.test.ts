import assert from 'node:assert/strict';
import test from 'node:test';
import {
  attemptMutation,
  attemptRead,
  reportMutationOutcome,
} from '../src/commands/command-policy';

const failure = (code: string, ambiguous = false) => ({
  code,
  message: code,
  ambiguous,
  retryable: true,
  fatal: false,
});

test('retryable ambiguous mutations make one attempt and require reconciliation', async () => {
  let writes = 0;
  let refreshes = 0;
  const result = await attemptMutation({ kind: 'mutation' }, async () => {
    writes++;
    throw failure('io', true);
  }, async () => { refreshes++; });
  assert.equal(writes, 1);
  assert.equal(refreshes, 0);
  assert.equal(result.outcome, 'unknown');
  if (result.outcome !== 'applied') assert.equal(result.recovery, 'reconcile');
});

test('resumable failures never submit or resume automatically', async () => {
  let attempts = 0;
  const result = await attemptMutation(
    { kind: 'resumable', operation: 'member-addition' },
    async () => {
      attempts++;
      throw failure('agent-lost');
    },
    async () => assert.fail('failed mutation was reported as applied'),
  );
  assert.equal(attempts, 1);
  assert.equal(result.outcome, 'unknown');
  if (result.outcome !== 'applied') assert.equal(result.recovery, 'explicit-resume');
});

test('definitive rejection and admission refusal remain distinct', async () => {
  for (const [error, expected] of [
    [failure('conflict'), 'rejected'],
    [{ ...failure('busy'), details: { reason: 'admission-not-started' } }, 'not-started'],
  ] as const) {
    let attempts = 0;
    const result = await attemptMutation({ kind: 'mutation' }, async () => {
      attempts++;
      throw error;
    }, async () => {});
    assert.equal(result.outcome, expected);
    assert.equal(attempts, 1);
  }
});

test('confirmed application with failed refresh cannot reach the mutation failure handler', async () => {
  let writes = 0;
  let pending = 0;
  const result = await attemptMutation({ kind: 'mutation' }, async () => {
    writes++;
    return 'confirmed';
  }, async () => { throw failure('profile-busy'); });
  assert.equal(result.outcome, 'applied');
  if (result.outcome === 'applied')
    assert.equal(result.synchronization, 'pending');
  assert.equal(await reportMutationOutcome(
    result,
    () => assert.fail('confirmed write was rejected'),
    () => { pending++; },
  ), true);
  assert.equal(writes, 1);
  assert.equal(pending, 1);
});

test('an onApplied callback that already handled refresh failure remains successful', async () => {
  const result = await attemptMutation(
    { kind: 'mutation' },
    async () => 7,
    async () => {},
  );
  assert.equal(result.outcome, 'applied');
  if (result.outcome === 'applied')
    assert.equal(result.synchronization, 'current');
  await reportMutationOutcome(result, () => assert.fail(), () => assert.fail());
});

test('read recovery is explicit and cannot replay a retryable read automatically', async () => {
  let reads = 0;
  const result = await attemptRead({ kind: 'read-recovery' }, async () => {
    reads++;
    throw failure('io');
  });
  assert.equal(reads, 1);
  assert.equal(result.outcome, 'read-failed');
  if (result.outcome === 'read-failed') assert.equal(result.recovery, 'read-recovery');
  assert.deepEqual(await attemptRead({ kind: 'read-recovery' }, async () => 3), {
    outcome: 'read', value: 3,
  });
});
