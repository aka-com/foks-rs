import { invoke } from '@tauri-apps/api/core';
import {
  isAgentSessionError,
  normalizeCommandError,
  normalizeMutationError,
  reportReadinessError,
  type CommandError,
} from './errors';

let nativeAgentGeneration = 0;

export function checkedMutation<T>(
  command: string,
  args: Record<string, unknown> | undefined,
  decode: (value: unknown) => T,
): Promise<T> {
  return checked(command, args, decode, true, true);
}

export async function checked<T>(
  command: string,
  args: Record<string, unknown> | undefined,
  decode: (value: unknown) => T,
  reportReadiness = true,
  mutation = false,
): Promise<T> {
  const generation = nativeAgentGeneration;
  let origin: 'invoke' | 'response' = 'invoke';
  try {
    const response = await invoke<unknown>(command, args);
    origin = 'response';
    const value = decode(response);
    if (
      command === 'auto_recover_agent' ||
      command === 'retry_agent_connection'
    )
      nativeAgentGeneration++;
    return value;
  } catch (error) {
    let typed = normalizeCommandError(error);
    typed = {
      ...typed,
      origin,
      ...(origin === 'response' && typed.code === 'invalid-command-error'
        ? { code: 'invalid-response' }
        : {}),
      ambiguous: mutation && typed.ambiguous,
    };
    if (mutation)
      typed =
        origin === 'response'
          ? { ...typed, ambiguous: true, retryable: false }
          : normalizeMutationError(typed);
    if (generation !== nativeAgentGeneration && isAgentSessionError(typed)) {
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
