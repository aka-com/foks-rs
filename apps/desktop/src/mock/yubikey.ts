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

/** One mock YubiKey enrollment, as `list_yubi_accounts` reports it. */
interface MockYubiEnrollment {
  alias: string;
  server: string;
  state: 'pending' | 'complete';
  serial?: number;
}

/**
 * The key alias a `runYubi` request names: its `alias`, or its `yubiAlias`
 * for the commands addressed by account store.
 */
function requestAlias(args: Record<string, unknown>): string {
  return 'alias' in args && typeof args.alias === 'string'
    ? args.alias
    : 'yubiAlias' in args && typeof args.yubiAlias === 'string'
      ? args.yubiAlias
      : 'primary key';
}

/**
 * One handler for each command `runYubi` sends, over the enrollments in
 * `yubi`. The record type requires a handler for every command, so a command
 * added to `YubiCommand` fails the build until it has its own response.
 */
function runYubiHandlers(
  yubi: MockYubiEnrollment[],
): Record<YubiCommand['command'], MockCommand> {
  const handler =
    (respond: (args: Record<string, unknown>) => unknown): MockCommand =>
    (args) =>
      respond(args ?? {});
  const pinStatus = handler(() => ({ remaining: 3, blocked: false }));
  const passphrase = handler(() => ({
    generation: 2,
    stretchVersion: 'v1',
    verified: true,
  }));
  const enrolled = (alias: string) => ({
    alias,
    username: 'satoshi',
    yubiId: `08${'8'.repeat(66)}`,
    subkeyId: `0d${'d'.repeat(64)}`,
    userChainSequence: 22,
    managementEnrolled: true,
  });
  const managementKey = handler((args) => ({
    alias: requestAlias(args),
    managementEnrolled: true,
    managementGeneration: 2,
  }));
  return {
    create_yubi_account: handler((args) => {
      const alias = requestAlias(args);
      yubi.push({
        alias,
        server:
          'profile' in args && typeof args.profile === 'string'
            ? args.profile
            : '',
        state: 'complete',
      });
      return enrolled(alias);
    }),
    resume_yubi_account: handler((args) => enrolled(requestAlias(args))),
    provision_yubi_device: handler((args) => enrolled(requestAlias(args))),
    sync_yubi_account: handler(() => ({
      username: 'satoshi',
      userChainSequence: 22,
      directories: 2,
      entries: 7,
      federation: [],
    })),
    yubi_pin_status: pinStatus,
    change_yubi_pin: pinStatus,
    set_yubi_passphrase: passphrase,
    change_yubi_passphrase: passphrase,
    verify_yubi_passphrase: passphrase,
    change_yubi_puk: handler((args) => ({
      alias: requestAlias(args),
      changed: true,
    })),
    unblock_yubi_pin: pinStatus,
    rotate_yubi_management_key: managementKey,
    resume_yubi_management_key: managementKey,
    recover_yubi_management_key: managementKey,
    recover_yubi_subkey: handler((args) => ({
      alias: requestAlias(args),
      subkeyId: `0d${'d'.repeat(64)}`,
      certificateCount: 2,
    })),
    revoke_yubi_device: handler((args) => ({
      alias: requestAlias(args),
      userChainSequence: 23,
      removedLocalCredential: true,
    })),
  };
}

/** The commands `runYubi` sends, each of which the mock host answers. */
export const MOCK_YUBI_COMMANDS = Object.keys(
  runYubiHandlers([]),
) as YubiCommand['command'][];

/**
 * The commands of `YUBI_HOST_MEMBERS`, answered with wire JSON: the card and
 * enrollment listings, and the handlers of `runYubiHandlers`.
 */
export function mockYubiHost({
  snapshot,
}: MockState): Record<string, MockCommand> {
  // `list_yubi_accounts` returns results for a single profile. The mock tracks
  // the associated server and enrollment status per account, filtering
  // enrollments by account and marking incomplete enrollments as pending.
  const yubi: MockYubiEnrollment[] = snapshot.yubiAccounts.map((entry) => ({
    alias: entry.alias,
    server: entry.server,
    state: entry.state,
    serial: entry.serial,
  }));
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
    ...runYubiHandlers(yubi),
  };
}
