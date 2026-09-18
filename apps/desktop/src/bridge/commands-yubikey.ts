import type { Bridge } from './contract';
import { checked } from './transport';
import { decodeYubiAccounts, decodeYubiCards, decodeYubiResult } from './yubikey';

export const yubiCommands: Pick<
  Bridge,
  'listYubiCards' | 'listYubiAccounts' | 'runYubi'
> = {
  listYubiCards: (profile) =>
    checked('list_yubi_cards', { profile }, decodeYubiCards),
  listYubiAccounts: (profile) =>
    checked('list_yubi_accounts', { profile }, decodeYubiAccounts),
  runYubi: ({ command, args }) =>
    checked(command, { ...args }, (value) => decodeYubiResult(command, value)),
};
