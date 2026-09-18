import {
  normalizeCommandError,
  normalizeMutationError,
  type CommandError,
} from './bridge';

export type FirstRunOperation =
  | 'pending-read'
  | 'server-check'
  | 'account-signup'
  | 'account-copy'
  | 'account-recovery'
  | 'account-pairing'
  | 'backup-prepare'
  | 'backup-commit'
  | 'passphrase'
  | 'group-discovery';

export interface FirstRunFailure {
  readonly operation: FirstRunOperation;
  readonly error: CommandError;
  readonly recovery: 'none' | 'pending' | 'reconciled' | 'failed';
  readonly recoveryError?: CommandError;
}

/** Uses structured error fields to determine whether reconciliation is required. */
export function classifyFirstRunFailure(
  operation: FirstRunOperation,
  error: unknown,
): FirstRunFailure {
  const typed = ['pending-read', 'backup-prepare', 'group-discovery'].includes(
    operation,
  )
    ? normalizeCommandError(error)
    : normalizeMutationError(error);
  const uncertain =
    typed.ambiguous ||
    typed.code === 'ambiguous' ||
    typed.code === 'response-binding';
  return {
    operation,
    error: typed,
    recovery: uncertain ? 'pending' : 'none',
  };
}

/** Reconciles uncertain state without replaying the operation that produced it. */
export async function reconcileFirstRunFailure(
  failure: FirstRunFailure,
  reconcile: () => Promise<void>,
): Promise<FirstRunFailure> {
  if (failure.recovery !== 'pending') return failure;
  try {
    await reconcile();
    return { ...failure, recovery: 'reconciled' };
  } catch (error) {
    return {
      ...failure,
      recovery: 'failed',
      recoveryError: normalizeCommandError(error),
    };
  }
}

export interface FirstRunFailurePresentation {
  readonly title: string;
  readonly detail: string;
  /** The raw error chain the agent kept out of the message, for reports. */
  readonly reason?: string;
}

/** Pure presentation for a retained, operation-scoped failure. */
export function presentFirstRunFailure(
  failure: FirstRunFailure,
): FirstRunFailurePresentation {
  const base = describeFirstRunFailure(failure);
  const reason = failure.error.details?.reason;
  return reason && reason !== failure.error.message
    ? { ...base, reason }
    : base;
}

function describeFirstRunFailure(
  failure: FirstRunFailure,
): FirstRunFailurePresentation {
  if (failure.recovery === 'pending') {
    return {
      title: failure.error.message,
      detail:
        'Checking whether the change was saved… FOKS will not try the change again automatically.',
    };
  }
  if (failure.recovery === 'reconciled') {
    return {
      title: failure.error.message,
      detail:
        'FOKS checked the latest setup state. Review it before trying again; the change was not repeated.',
    };
  }
  if (failure.recovery === 'failed') {
    return {
      title: failure.error.message,
      detail: `FOKS could not check whether the change was saved: ${failure.recoveryError?.message ?? 'the refresh failed'}`,
    };
  }
  if (
    failure.error.code === 'bootstrap-required' ||
    failure.error.code === 'agent-lost'
  ) {
    return {
      title: failure.error.message,
      detail:
        'Restore the local agent, then review the current setup state before retrying.',
    };
  }
  return {
    title: failure.error.message,
    detail:
      'Review the reported error and the server address before trying again. FOKS cannot tell from this error alone whether anything changed.',
  };
}
