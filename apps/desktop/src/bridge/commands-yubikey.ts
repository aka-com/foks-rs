import type { Bridge } from './contract';
import type { Transport } from './transport';
import {
  decodeYubiAccounts,
  decodeYubiCards,
  decodeYubiResult,
  type YubiCommand,
} from './yubikey';

/**
 * The YubiKey commands that only read. `runYubi` sends every other command as
 * a mutation, whose failure with an unknown outcome is reported as ambiguous.
 */
export const YUBI_READ_COMMANDS: readonly YubiCommand['command'][] = [
  'yubi_pin_status',
  'verify_yubi_passphrase',
];

export function yubiCommandMutates(command: YubiCommand['command']): boolean {
  return !YUBI_READ_COMMANDS.includes(command);
}

export const yubiCommands = (
  transport: Transport,
): Pick<Bridge, 'listYubiCards' | 'listYubiAccounts' | 'runYubi'> => ({
  listYubiCards: (profile) =>
    transport.checked('list_yubi_cards', { profile }, decodeYubiCards),
  listYubiAccounts: (profile) =>
    transport.checked('list_yubi_accounts', { profile }, decodeYubiAccounts),
  runYubi: ({ command, args }) =>
    (yubiCommandMutates(command)
      ? transport.checkedMutation
      : transport.checked)(command, { ...args }, (value) =>
      decodeYubiResult(command, value),
    ),
});
