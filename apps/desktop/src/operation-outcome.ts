import { normalizeCommandError } from './bridge';
import type { CommandError } from './bridge';

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
  if (
    [
      'profile-busy',
      'mutation-in-flight',
      'catalog-required',
      'bootstrap-required',
    ].includes(typed.code)
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
