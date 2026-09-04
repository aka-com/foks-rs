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
  if (normalizeCommandError(error).code === 'agent-lost') return;
  try {
    await refresh();
  } catch (refreshError) {
    reportRefreshError(refreshError);
  }
}
