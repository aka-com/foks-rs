import { invoke } from '@tauri-apps/api/core';
import {
  isAgentReadinessError,
  normalizeCommandError,
  reportReadinessError,
  type CommandError,
} from './errors';

let nativeAgentGeneration = 0;

export async function checked<T>(
  command: string,
  args: Record<string, unknown> | undefined,
  decode: (value: unknown) => T,
  reportReadiness = true,
): Promise<T> {
  const generation = nativeAgentGeneration;
  try {
    const value = decode(await invoke<unknown>(command, args));
    if (
      command === 'auto_recover_agent' ||
      command === 'retry_agent_connection'
    )
      nativeAgentGeneration++;
    return value;
  } catch (error) {
    const typed = normalizeCommandError(error);
    if (generation !== nativeAgentGeneration && isAgentReadinessError(typed)) {
      throw {
        ...typed,
        code: 'agent-request-retired',
        message: 'An earlier agent request was retired after recovery.',
        retryable: false,
        fatal: false,
      } satisfies CommandError;
    }
    if (reportReadiness) reportReadinessError(typed);
    throw typed;
  }
}
