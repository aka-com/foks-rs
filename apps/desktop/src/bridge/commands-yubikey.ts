import type { Bridge } from './contract';
import { checked, checkedMutation } from './transport';
import {
  decodeYubiAccounts,
  decodeYubiCards,
  decodeYubiResult,
} from './yubikey';

export const yubiCommands: Pick<
  Bridge,
  'listYubiCards' | 'listYubiAccounts' | 'runYubi'
> = {
  listYubiCards: (profile) =>
    checked('list_yubi_cards', { profile }, decodeYubiCards),
  listYubiAccounts: (profile) =>
    checked('list_yubi_accounts', { profile }, decodeYubiAccounts),
  runYubi: ({ command, args }) =>
    (['yubi_pin_status', 'verify_yubi_passphrase'].includes(command)
      ? checked
      : checkedMutation)(command, { ...args }, (value) =>
      decodeYubiResult(command, value),
    ),
};
