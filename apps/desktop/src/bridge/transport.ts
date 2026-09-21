import { invoke } from '@tauri-apps/api/core';
import {
  diagnosticLog,
  hashId,
  outcomeForCode,
  type TimingValue,
} from '../diagnostics/log';
import {
  isAgentSessionError,
  normalizeCommandError,
  normalizeMutationError,
  reportReadinessError,
  type CommandError,
} from './errors';

let nativeAgentGeneration = 0;

/**
 * The scope a command's arguments name, when they name one the Refresh status
 * popover already prints: a profile (a server id), or a store, recorded as a
 * hash because a store id spells out the account and team aliases. Nothing
 * else in the arguments is read.
 */
function commandScope(
  args: Record<string, unknown> | undefined,
): string | undefined {
  const profile = args?.profile;
  if (typeof profile === 'string') return profile;
  for (const key of ['storeId', 'store', 'accountStoreId'] as const) {
    const value = args?.[key];
    if (typeof value === 'string') return `store#${hashId(value)}`;
  }
  return undefined;
}

function commandAttrs(
  command: string,
  args: Record<string, unknown> | undefined,
): Record<string, TimingValue> {
  const action = args?.action;
  return {
    command,
    ...(action && typeof action === 'object' && 'action' in action
      ? { action: String(action.action) }
      : {}),
  };
}

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
  const end = diagnosticLog.span('invoke', {
    scope: commandScope(args),
    attrs: commandAttrs(command, args),
  });
  try {
    const response = await invoke<unknown>(command, args);
    origin = 'response';
    const value = decode(response);
    if (
      command === 'auto_recover_agent' ||
      command === 'retry_agent_connection'
    )
      nativeAgentGeneration++;
    end('ok');
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
    // Recorded after the code the caller will see is settled, so a request
    // the renderer itself retired is not counted as a command that failed.
    if (generation !== nativeAgentGeneration && isAgentSessionError(typed)) {
      const retired = {
        ...typed,
        code: 'agent-request-retired',
        message: 'An earlier agent request was retired after recovery.',
        retryable: false,
        fatal: false,
      } satisfies CommandError;
      end(outcomeForCode(retired.code), { code: retired.code });
      throw retired;
    }
    end(outcomeForCode(typed.code), { code: typed.code });
    if (reportReadiness) reportReadinessError(typed);
    throw typed;
  }
}
