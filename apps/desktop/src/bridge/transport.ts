import { Channel, invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
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

/** A channel the host sends messages on while one command runs. */
export interface TransportChannel {
  onmessage: (message: unknown) => void;
}

/**
 * What a transport needs from its host: Tauri's IPC in the application, or a
 * wire-level stand-in during development and render tests. `invoke` resolves
 * with the command's response as the host serialized it, or rejects with the
 * error the host returned; `listen` delivers each event's payload.
 */
export interface TransportHost {
  invoke: (
    command: string,
    args: Record<string, unknown> | undefined,
  ) => Promise<unknown>;
  listen: (
    event: string,
    handler: (payload: unknown) => void,
  ) => Promise<() => void>;
  channel: () => TransportChannel;
}

/**
 * The calls the bridge's commands make. `checked` decodes a response and
 * normalizes a failure, recording whether it arose before or after the host
 * responded; `checkedMutation` also marks a failure whose outcome is not known
 * as ambiguous. `invoke` is for the few commands whose response is not read.
 */
export interface Transport {
  invoke: (command: string, args?: Record<string, unknown>) => Promise<void>;
  checked: <T>(
    command: string,
    args: Record<string, unknown> | undefined,
    decode: (value: unknown) => T,
    reportReadiness?: boolean,
    mutation?: boolean,
  ) => Promise<T>;
  checkedMutation: <T>(
    command: string,
    args: Record<string, unknown> | undefined,
    decode: (value: unknown) => T,
  ) => Promise<T>;
  listen: TransportHost['listen'];
  channel: TransportHost['channel'];
}

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

export function createTransport(host: TransportHost): Transport {
  let agentGeneration = 0;
  const transport: Transport = {
    invoke: async (command, args) => {
      await host.invoke(command, args);
    },
    checkedMutation: (command, args, decode) =>
      transport.checked(command, args, decode, true, true),
    checked: async (
      command,
      args,
      decode,
      reportReadiness = true,
      mutation = false,
    ) => {
      const generation = agentGeneration;
      let origin: 'invoke' | 'response' = 'invoke';
      const end = diagnosticLog.span('invoke', {
        scope: commandScope(args),
        attrs: commandAttrs(command, args),
      });
      try {
        const response = await host.invoke(command, args);
        origin = 'response';
        const value = decode(response);
        if (
          command === 'auto_recover_agent' ||
          command === 'retry_agent_connection'
        )
          agentGeneration++;
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
        // Recorded after the code the caller will see is settled, so a
        // request the renderer itself retired is not counted as a command
        // that failed.
        if (generation !== agentGeneration && isAgentSessionError(typed)) {
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
    },
    listen: (event, handler) => host.listen(event, handler),
    channel: () => host.channel(),
  };
  return transport;
}

/** The transport over Tauri's IPC, which the native bridge uses. */
export const tauriTransport: Transport = createTransport({
  invoke: (command, args) => invoke<unknown>(command, args),
  listen: (event, handler) =>
    listen<unknown>(event, (message) => handler(message.payload)),
  channel: () => new Channel<unknown>(),
});
