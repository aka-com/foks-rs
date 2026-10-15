/**
 * The mock's YubiKey commands, the members of `bridge/commands-yubikey.ts`,
 * which answer at the wire level through the mock host from the fixture's
 * cards and enrolled accounts.
 */

import type { yubiCommands } from '../bridge/commands-yubikey';
import type { YubiCommand } from '../bridge/yubikey';
import { mockCommand, type MockCommand } from './host';
import type { MockState } from './state';

/** The YubiKey members whose commands the mock host answers: all of them. */
export const YUBI_HOST_MEMBERS = [
  'listYubiCards',
  'listYubiAccounts',
  'runYubi',
] as const satisfies readonly (keyof ReturnType<typeof yubiCommands>)[];

/** Each command `runYubi` sends; the record type requires every one. */
const RUN_YUBI_COMMANDS: Readonly<Record<YubiCommand['command'], true>> = {
  create_yubi_account: true,
  resume_yubi_account: true,
  provision_yubi_device: true,
  sync_yubi_account: true,
  yubi_pin_status: true,
  change_yubi_pin: true,
  set_yubi_passphrase: true,
  change_yubi_passphrase: true,
  verify_yubi_passphrase: true,
  change_yubi_puk: true,
  unblock_yubi_pin: true,
  rotate_yubi_management_key: true,
  resume_yubi_management_key: true,
  recover_yubi_management_key: true,
  recover_yubi_subkey: true,
  revoke_yubi_device: true,
};

/** The commands `runYubi` sends, each of which the mock host answers. */
export const MOCK_YUBI_COMMANDS = Object.keys(
  RUN_YUBI_COMMANDS,
) as YubiCommand['command'][];

/**
 * The commands of `YUBI_HOST_MEMBERS`, answered with wire JSON: the card and
 * enrollment listings, and one handler per command `runYubi` sends.
 */
export function mockYubiHost({
  snapshot,
}: MockState): Record<string, MockCommand> {
  // `list_yubi_accounts` returns results for a single profile. The mock tracks
  // the associated server and enrollment status per account, filtering
  // enrollments by account and marking incomplete enrollments as pending.
  const yubi: {
    alias: string;
    server: string;
    state: 'pending' | 'complete';
    serial?: number;
  }[] = snapshot.yubiAccounts.map((entry) => ({
    alias: entry.alias,
    server: entry.server,
    state: entry.state,
    serial: entry.serial,
  }));
  const run = (
    command: YubiCommand['command'],
    args: Record<string, unknown>,
  ): unknown => {
    if (
      command === 'yubi_pin_status' ||
      command === 'change_yubi_pin' ||
      command === 'unblock_yubi_pin'
    )
      return { remaining: 3, blocked: false };
    if (command.includes('passphrase'))
      return { generation: 2, stretchVersion: 'v1', verified: true };
    const alias =
      'alias' in args && typeof args.alias === 'string'
        ? args.alias
        : 'yubiAlias' in args && typeof args.yubiAlias === 'string'
          ? args.yubiAlias
          : 'primary key';
    if (command === 'create_yubi_account')
      yubi.push({
        alias,
        server:
          'profile' in args && typeof args.profile === 'string'
            ? args.profile
            : '',
        state: 'complete',
      });
    if (
      command === 'create_yubi_account' ||
      command === 'resume_yubi_account' ||
      command === 'provision_yubi_device'
    ) {
      return {
        alias,
        username: 'satoshi',
        yubiId: `08${'8'.repeat(66)}`,
        subkeyId: `0d${'d'.repeat(64)}`,
        userChainSequence: 22,
        managementEnrolled: true,
      };
    }
    if (command === 'sync_yubi_account')
      return {
        username: 'satoshi',
        userChainSequence: 22,
        directories: 2,
        entries: 7,
        federation: [],
      };
    if (command === 'change_yubi_puk') return { alias, changed: true };
    if (command === 'recover_yubi_subkey')
      return { alias, subkeyId: `0d${'d'.repeat(64)}`, certificateCount: 2 };
    if (command === 'revoke_yubi_device')
      return { alias, userChainSequence: 23, removedLocalCredential: true };
    return { alias, managementEnrolled: true, managementGeneration: 2 };
  };
  return {
    list_yubi_cards: () => snapshot.cardsConnected.map((card) => ({ ...card })),
    list_yubi_accounts: mockCommand(({ profile }: { profile: string }) =>
      yubi
        .filter((entry) => entry.server === profile)
        .map((entry) => ({
          alias: entry.alias,
          state: entry.state,
          cardSerial: entry.serial,
        })),
    ),
    ...Object.fromEntries(
      MOCK_YUBI_COMMANDS.map((command): [string, MockCommand] => [
        command,
        (args) => run(command, args ?? {}),
      ]),
    ),
  };
}
