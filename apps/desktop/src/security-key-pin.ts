/**
 * Security-key PIN and PUK rules, and the agent refusals a PIN entry can
 * produce.
 *
 * The rules mirror `foks_yubi::Pin::new` (six to eight printable ASCII
 * characters, for a PIN and a PUK alike) and
 * `foks_yubi::PinRetryConfiguration::new` (one to fifteen attempts). The
 * fixture `crates/foks-yubi/tests/fixtures/pin-policy.json` is checked by
 * both the Rust crate and the desktop tests, so the two cannot drift apart.
 *
 * The agent reports a rejected or blocked PIN as `operation-failed` with the
 * `foks_yubi::Error` text as its message; there is no dedicated error code.
 * `securityKeyPinRefusal` recognizes those messages so that a prompt can show
 * the remaining attempts instead of asking for the PIN as if nothing had been
 * tried.
 */

import { normalizeCommandError } from './bridge';

/** Characters in a PIN or PUK, as `foks_yubi::Pin::new` accepts. */
export const SECURITY_KEY_PIN_LENGTH = { min: 6, max: 8 } as const;

/** Attempts a card may allow for its PIN or PUK, set during enrollment. */
export const SECURITY_KEY_RETRY_RANGE = { min: 1, max: 15 } as const;

/**
 * Why `value` cannot be submitted as a PIN or PUK, or null when it can.
 * An empty value is not a problem here: whether a PIN is required is the
 * caller's decision.
 */
export function securityKeyPinProblem(value: string): string | null {
  if (!value) return null;
  // Printable ASCII without the space: U+0021 to U+007E, as
  // `u8::is_ascii_graphic` accepts.
  if (!/^[\x21-\x7e]*$/.test(value))
    return 'Use letters, digits and symbols only, without spaces.';
  if (
    value.length < SECURITY_KEY_PIN_LENGTH.min ||
    value.length > SECURITY_KEY_PIN_LENGTH.max
  )
    return `Use ${SECURITY_KEY_PIN_LENGTH.min} to ${SECURITY_KEY_PIN_LENGTH.max} characters.`;
  return null;
}

/** Whether `value` is a PIN or PUK the card can be given. */
export function validSecurityKeyPin(value: string): boolean {
  return value !== '' && securityKeyPinProblem(value) === null;
}

/** Whether `value` is a retry count enrollment can set. */
export function validSecurityKeyRetries(value: number): boolean {
  return (
    Number.isInteger(value) &&
    value >= SECURITY_KEY_RETRY_RANGE.min &&
    value <= SECURITY_KEY_RETRY_RANGE.max
  );
}

/** A PIN the card refused, as reported by the agent. */
export type SecurityKeyPinRefusal =
  { kind: 'rejected'; remaining: number } | { kind: 'blocked' };

/**
 * The PIN refusal that `error` reports, or null. `error` is a command
 * failure or the message text a prompt already shows.
 */
export function securityKeyPinRefusal(
  error: unknown,
): SecurityKeyPinRefusal | null {
  if (error === null || error === undefined || error === '') return null;
  const message =
    typeof error === 'string' ? error : normalizeCommandError(error).message;
  // `foks_yubi::Error::PinRejected` and `PinBlocked`.
  const rejected =
    /hardware key PIN was rejected; (\d+) attempt\(s\) remain/i.exec(message);
  if (rejected) {
    const remaining = Number(rejected[1]);
    return remaining === 0
      ? { kind: 'blocked' }
      : { kind: 'rejected', remaining };
  }
  if (/hardware key PIN is blocked/i.test(message)) return { kind: 'blocked' };
  return null;
}

/** The note a PIN field shows under its input after a refusal. */
export function securityKeyPinRefusalText(
  refusal: SecurityKeyPinRefusal,
): string {
  if (refusal.kind === 'blocked')
    return 'The PIN is blocked. Unblock it with the PUK in Settings › Server › Security keys › Unblock PIN…, then enter the new PIN here.';
  if (refusal.remaining === 1)
    return '1 attempt left. Another incorrect PIN blocks the PIN until it is unblocked with the PUK.';
  return `${refusal.remaining} attempts left before the PIN is blocked.`;
}
