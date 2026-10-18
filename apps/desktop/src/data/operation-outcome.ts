import { normalizeCommandError } from '../bridge';
import type { CommandError } from '../bridge';

/** A confirmed write stays applied even when its read model cannot be loaded. */
export type AppliedOutcome<T> =
  | { outcome: 'applied'; synchronization: 'current'; value: T }
  | { outcome: 'applied'; synchronization: 'pending'; error: CommandError };

export async function synchronizeApplied<T>(
  read: () => Promise<T>,
): Promise<AppliedOutcome<T>> {
  try {
    return {
      outcome: 'applied',
      synchronization: 'current',
      value: await read(),
    };
  } catch (error) {
    return {
      outcome: 'applied',
      synchronization: 'pending',
      error: normalizeCommandError(error),
    };
  }
}

/** Only typed admission failures establish that execution never started. */
export function failureOutcome(
  error: unknown,
): 'not-started' | 'unknown' | 'rejected' {
  const typed = normalizeCommandError(error);
  if (typed.ambiguous) return 'unknown';
  if (typed.details?.reason === 'admission-not-started') return 'not-started';
  // A callback can itself report contention after execution has begun.
  if (typed.code === 'busy' || typed.code === 'profile-busy') return 'unknown';
  // A clock failure can follow a dispatched operation body, and a failed
  // lock-state check can follow a completed write.
  if (
    [
      'deadline-clock',
      'deadline-clock-unavailable',
      'deadline-domain-mismatch',
      'missing-deadline',
      'app-lock-unavailable',
    ].includes(typed.code)
  )
    return 'unknown';
  // CommandState refuses a command without a deadline policy before its body.
  if (typed.code === 'missing-deadline-policy') return 'not-started';
  // Upload admission refused the upload; it needs reconciliation, cleanup or
  // clock repair before another attempt.
  if (
    [
      'upload-expired',
      'upload-identity-conflict',
      'upload-generation-mismatch',
      'upload-legacy-read-only',
      'upload-pending-full',
      'upload-retention-full',
      'upload-spool-full',
      'upload-clock-untrusted',
      'upload-future',
      'authentication-interrupted',
    ].includes(typed.code)
  )
    return 'rejected';
  if (
    ['mutation-in-flight', 'catalog-required', 'bootstrap-required'].includes(
      typed.code,
    )
  )
    return 'not-started';
  // Unknown transport/worker errors must not imply that a new submission is safe.
  if (
    ['agent-lost', 'io', 'unknown', 'invalid-command-error'].includes(
      typed.code,
    )
  )
    return 'unknown';
  return 'rejected';
}
