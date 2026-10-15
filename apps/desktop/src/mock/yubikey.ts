/**
 * The mock's YubiKey commands, the members of `bridge/commands-yubikey.ts`,
 * which answer from the fixture's cards and enrolled accounts with decoded
 * values.
 */

import type { yubiCommands } from '../bridge/commands-yubikey';
import type { MockDomain, MockState } from './state';

export function mockYubiCommands({
  snapshot,
}: MockState): MockDomain<typeof yubiCommands> {
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
  return {
    listYubiCards: async () =>
      snapshot.cardsConnected.map((card) => ({ ...card })),
    listYubiAccounts: async (profile) =>
      yubi
        .filter((entry) => entry.server === profile)
        .map((entry) => ({
          alias: entry.alias,
          state: entry.state,
          cardSerial: entry.serial,
        })),
    runYubi: async ({ command, args }) => {
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
    },
  };
}
