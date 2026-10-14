import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { hardwareUnlockRequested } from '../src/invitation-writes';
import {
  SECURITY_KEY_RETRY_RANGE,
  securityKeyPinProblem,
  securityKeyPinRefusal,
  securityKeyPinRefusalText,
  validSecurityKeyPin,
  validSecurityKeyRetries,
} from '../src/security-key-pin';

interface PinPolicyFixture {
  pin: { accepted: string[]; rejected: string[] };
  retries: { min: number; max: number };
  refusals: { message: string; kind: string; remaining?: number }[];
}

const fixture = JSON.parse(
  readFileSync(
    new URL(
      '../../../crates/foks-yubi/tests/fixtures/pin-policy.json',
      import.meta.url,
    ),
    'utf8',
  ),
) as PinPolicyFixture;

/** The agent's message for a failure it has no code for (`sentence`). */
function agentFailure(message: string) {
  return {
    code: 'operation-failed',
    message: message.charAt(0).toUpperCase() + message.slice(1),
    retryable: false,
    ambiguous: false,
    fatal: false,
  };
}

test('the PIN rule matches the fixture also checked by foks-yubi', () => {
  for (const value of fixture.pin.accepted) {
    assert.equal(securityKeyPinProblem(value), null, value);
    assert.equal(validSecurityKeyPin(value), true, value);
  }
  for (const value of fixture.pin.rejected) {
    assert.equal(validSecurityKeyPin(value), false, JSON.stringify(value));
    if (value)
      assert.notEqual(
        securityKeyPinProblem(value),
        null,
        JSON.stringify(value),
      );
  }
  // An empty PIN is the caller's to require; it is not a malformed one.
  assert.equal(securityKeyPinProblem(''), null);
});

test('the retry range matches the fixture also checked by foks-yubi', () => {
  assert.deepEqual(SECURITY_KEY_RETRY_RANGE, fixture.retries);
  const { min, max } = fixture.retries;
  assert.equal(validSecurityKeyRetries(min), true);
  assert.equal(validSecurityKeyRetries(max), true);
  assert.equal(validSecurityKeyRetries(min - 1), false);
  assert.equal(validSecurityKeyRetries(max + 1), false);
  assert.equal(validSecurityKeyRetries(2.5), false);
});

test('agent PIN refusals are recognized with their remaining attempts', () => {
  for (const { message, kind, remaining } of fixture.refusals) {
    const expected =
      kind === 'blocked' ? { kind } : { kind, remaining: remaining! };
    assert.deepEqual(securityKeyPinRefusal(agentFailure(message)), expected);
    assert.deepEqual(securityKeyPinRefusal(message), expected);
  }
  assert.equal(securityKeyPinRefusal('Hardware key PIN is required'), null);
  assert.equal(securityKeyPinRefusal(null), null);
  assert.match(
    securityKeyPinRefusalText({ kind: 'rejected', remaining: 1 }),
    /^1 attempt left\. Another incorrect PIN blocks the PIN/,
  );
  assert.equal(
    securityKeyPinRefusalText({ kind: 'rejected', remaining: 4 }),
    '4 attempts left before the PIN is blocked.',
  );
  assert.match(
    securityKeyPinRefusalText({ kind: 'blocked' }),
    /^The PIN is blocked\. Unblock it with the PUK/,
  );
});

test('a refused or malformed PIN is not reported as a request for the PIN', () => {
  for (const { message } of fixture.refusals)
    assert.equal(hardwareUnlockRequested(agentFailure(message)), false);
  assert.equal(
    hardwareUnlockRequested(
      agentFailure(
        'hardware key management policy rejected the operation: PIN or PUK must contain six to eight printable ASCII characters',
      ),
    ),
    false,
  );
  assert.equal(
    hardwareUnlockRequested(agentFailure('hardware key PIN is required')),
    true,
  );
  assert.equal(
    hardwareUnlockRequested(
      agentFailure(
        'FOKS security refresh is deferred until a hardware key is unlocked: unlock the account key for invitations',
      ),
    ),
    true,
  );
});
