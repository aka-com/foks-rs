import { failureOutcome } from './operation-outcome';
import { normalizeCommandError } from './bridge';
import type { Item } from './model';

export interface MutationFailureOptions {
  item?: Item;
  draft?: string;
  report?: boolean;
}

export type MutationFailureHandler = (
  error: unknown,
  options?: MutationFailureOptions,
) => Promise<void>;

export async function reconcileMutationFailure(
  error: unknown,
  refresh: () => Promise<void>,
  reportRefreshError: (error: unknown) => void,
): Promise<void> {
  const typed = normalizeCommandError(error);
  if (typed.code === 'agent-lost') return;
  // A refused concurrent write did not retire the catalog; refreshing here
  // would compete with the operation that still owns the mutation permit.
  if (
    failureOutcome(typed) === 'not-started' &&
    typed.code === 'mutation-in-flight'
  )
    return;
  try {
    await refresh();
  } catch (refreshError) {
    reportRefreshError(refreshError);
  }
}
