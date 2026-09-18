import { normalizeCommandError } from '../bridge';
import type { CommandError } from '../bridge';
import { failureOutcome, synchronizeApplied } from '../operation-outcome';
import type { AppliedOutcome } from '../operation-outcome';

export type CommandPolicy =
  | { kind: 'read-recovery' }
  | { kind: 'mutation' }
  | { kind: 'resumable'; operation: string };

export type MutationPolicy = Exclude<CommandPolicy, { kind: 'read-recovery' }>;

export type CommandOutcome<T> =
  | AppliedOutcome<T>
  | {
      outcome: 'not-started' | 'rejected' | 'unknown';
      error: CommandError;
      recovery: 'reconcile' | 'explicit-resume';
    };

export async function attemptMutation<T>(
  policy: MutationPolicy,
  write: () => Promise<T>,
  onApplied: (value: T) => Promise<void>,
): Promise<CommandOutcome<T>> {
  let value: T;
  try {
    value = await write();
  } catch (error) {
    return {
      outcome: failureOutcome(error),
      error: normalizeCommandError(error),
      recovery: policy.kind === 'resumable' ? 'explicit-resume' : 'reconcile',
    };
  }
  return synchronizeApplied(async () => {
    await onApplied(value);
    return value;
  });
}

export async function reportMutationOutcome<T>(
  result: CommandOutcome<T>,
  onFailure: (error: CommandError) => void | Promise<void>,
  onRefreshPending: (error: CommandError) => void | Promise<void>,
): Promise<boolean> {
  if (result.outcome !== 'applied') {
    await onFailure(result.error);
    return false;
  }
  if (result.synchronization === 'pending')
    await onRefreshPending(result.error);
  return true;
}

export async function attemptRead<T>(
  policy: Extract<CommandPolicy, { kind: 'read-recovery' }>,
  read: () => Promise<T>,
): Promise<
  | { outcome: 'read'; value: T }
  | { outcome: 'read-failed'; error: CommandError; recovery: typeof policy.kind }
> {
  try {
    return { outcome: 'read', value: await read() };
  } catch (error) {
    return {
      outcome: 'read-failed',
      error: normalizeCommandError(error),
      recovery: policy.kind,
    };
  }
}
