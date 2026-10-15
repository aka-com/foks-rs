/**
 * A wire-level stand-in for the native host. The commands the mock bridge
 * implements at the wire level answer here with the JSON a native command
 * returns, or throw the CommandError it rejects with, so the bridge's
 * transport decodes and normalizes them as it does the native host's answers.
 */

import type { TransportChannel, TransportHost } from '../bridge/transport';

/** One mock command: its Tauri arguments in, its wire response out. */
export type MockCommand = (
  args: Record<string, unknown> | undefined,
) => unknown;

/**
 * Wraps a handler that reads its arguments as `A`, the object the bridge's
 * command factory sends.
 */
export function mockCommand<A>(handler: (args: A) => unknown): MockCommand {
  return (args) => handler(args as A);
}

/** What crosses the IPC boundary: JSON, as Tauri serializes it. */
function serialized(value: unknown): unknown {
  return value === undefined
    ? null
    : (JSON.parse(JSON.stringify(value)) as unknown);
}

export function mockHost(
  commands: Readonly<Record<string, MockCommand>>,
): TransportHost {
  return {
    invoke: async (command, args) => {
      const handler = Object.hasOwn(commands, command)
        ? commands[command]
        : undefined;
      if (!handler)
        throw {
          code: 'invalid-request',
          message: `The mock host has no command ${command}.`,
          retryable: false,
          ambiguous: false,
          fatal: false,
        };
      try {
        return serialized(await handler(args));
      } catch (error) {
        throw serialized(error);
      }
    },
    listen: async () => () => undefined,
    channel: (): TransportChannel => ({ onmessage: () => undefined }),
  };
}
