/**
 * The mock's enrollment commands, the members of
 * `bridge/commands-enrollment.ts`: first-run account setup, recovery
 * phrases, devices, pairing, passphrases and profile reset. All but the two
 * native-only methods answer at the wire level through the mock host.
 */

import type { enrollmentCommands } from '../bridge/commands-enrollment';
import { mockCommand, type MockCommand } from './host';
import { failure, type MockDomain, type MockState } from './state';

/** The personal account's one recovery phrase: the key the fixture phrase derives. */
const FIXTURE_BACKUP_ID = `10${'4'.repeat(64)}`;

/**
 * The enrollment members whose commands the mock host answers at the wire
 * level, through `bridge/commands-enrollment.ts`: all of them except the two
 * that only the native host offers.
 */
export const ENROLLMENT_HOST_MEMBERS = [
  'discoverGoProfiles',
  'listPendingOperations',
  'createFirstRunAccount',
  'resumeFirstRunAccount',
  'setFirstRunPassphrase',
  'prepareOwnerBackup',
  'commitOwnerBackup',
  'recoverOwnerAccount',
  'resumeOwnerRecovery',
  'listAccountDevices',
  'removeAccountDevice',
  'listBackupEnrollments',
  'revokeOwnerBackup',
  'verifyRecoveryPhrase',
  'startDevicePairing',
  'resumeDevicePairingOffer',
  'finishDevicePairing',
  'acceptDevicePairing',
  'acceptGoProfilePairing',
  'resumeDevicePairingAcceptance',
  'resumeGoProfilePairing',
  'copyGoProfileDevice',
  'setAccountPassphrase',
  'changeAccountPassphrase',
  'verifyAccountPassphrase',
  'accountPassphraseStatus',
  'describeReset',
  'resetServer',
] as const satisfies readonly (keyof ReturnType<typeof enrollmentCommands>)[];

/**
 * The commands of `ENROLLMENT_HOST_MEMBERS`, answered with wire JSON over the
 * account stores in `state` and the device, backup, pairing and reset records
 * this function keeps.
 */
export function mockEnrollmentHost(
  state: MockState,
): Record<string, MockCommand> {
  const { snapshot, stores, firstRunFixture, ensureFirstRunAccount } = state;
  const pending = new Map<
    string,
    {
      kind: 'account-signup' | 'account-recovery';
      alias: string;
      target?: string;
    }[]
  >();
  const pairingOffers = new Map<string, string>();
  const pairingAcceptances = new Set<string>();
  const resetTokens = new Set<string>();
  const deviceRows = new Map<
    string,
    {
      id: string;
      name?: string;
      role: 'owner' | 'admin' | 'member';
      current: boolean;
    }[]
  >();
  const backupRows = new Map<
    string,
    { backupAlias: string; accountAlias: string; backupId: string }[]
  >();
  const accountStore = (id: string) =>
    stores.find((store) => store.id === id && store.kind === 'account');
  for (const store of stores) {
    if (store.kind !== 'account') continue;
    const rows =
      store.account === 'personal'
        ? [
            ...snapshot.devices.map((device) => ({
              id: device.id_hex.replace(/^02/, '04'),
              name: device.name,
              role: 'owner' as const,
              current: device.current,
            })),
            // Hardware security key device record (prefix `08`, 66 hex
            // digits after it), providing fixture data that distinguishes
            // card-bound keys from host keys.
            {
              id: `08${'5'.repeat(66)}`,
              name: 'Pocket YubiKey',
              role: 'member' as const,
              current: false,
            },
          ]
        : [
            {
              id: `04${'8'.repeat(64)}`,
              name: 'MacBook Pro',
              role: 'owner' as const,
              current: true,
            },
          ];
    deviceRows.set(store.id, rows);
    backupRows.set(
      store.id,
      store.account === 'personal'
        ? [
            {
              backupAlias: 'paper-backup',
              accountAlias: store.account,
              backupId: FIXTURE_BACKUP_ID,
            },
          ]
        : [],
    );
  }
  return {
    discover_go_profiles: () => ({ installed: false, candidates: [] }),
    list_pending_operations: mockCommand(({ profile }: { profile: string }) =>
      (pending.get(profile) ?? []).map((operation) => ({ ...operation })),
    ),
    create_first_run_account: mockCommand(
      async ({
        profile,
        alias,
        username,
      }: {
        profile: string;
        alias: string;
        username: string;
      }) => {
        pending.set(profile, [{ kind: 'account-signup', alias }]);
        ensureFirstRunAccount(profile, alias, username);
        pending.delete(profile);
        return { applied: true };
      },
    ),
    resume_first_run_account: mockCommand(
      async ({ profile, alias }: { profile: string; alias: string }) => {
        const operation = (pending.get(profile) ?? []).find(
          (entry) => entry.kind === 'account-signup' && entry.alias === alias,
        );
        if (!operation)
          throw failure(
            'pending-operation-not-found',
            'That account setup is no longer pending.',
          );
        pending.delete(profile);
        return { applied: true };
      },
    ),
    set_first_run_passphrase: () => ({ applied: true }),
    prepare_owner_backup: mockCommand(
      ({ backupAlias }: { backupAlias: string }) => ({
        backupAlias,
        phrase: firstRunFixture.recoveryPhrase,
      }),
    ),
    commit_owner_backup: mockCommand(
      async ({
        backupAlias,
        phrase,
      }: {
        backupAlias: string;
        phrase: string;
      }) => {
        if (
          backupAlias !== 'paper' ||
          phrase !== firstRunFixture.recoveryPhrase
        ) {
          throw failure(
            'invalid-request',
            'The recovery phrase does not match.',
          );
        }
        return { applied: true };
      },
    ),
    recover_owner_account: mockCommand(
      async ({
        profile,
        targetAlias,
      }: {
        profile: string;
        targetAlias: string;
      }) => {
        pending.set(profile, [
          { kind: 'account-recovery', alias: targetAlias },
        ]);
        const path =
          profile === firstRunFixture.invited.profile
            ? firstRunFixture.invited
            : firstRunFixture.own;
        ensureFirstRunAccount(profile, targetAlias, path.username);
        pending.delete(profile);
        return { applied: true };
      },
    ),
    resume_owner_recovery: mockCommand(
      async ({
        profile,
        targetAlias,
      }: {
        profile: string;
        targetAlias: string;
      }) => {
        const operation = (pending.get(profile) ?? []).find(
          (entry) =>
            entry.kind === 'account-recovery' && entry.alias === targetAlias,
        );
        if (!operation)
          throw failure(
            'pending-operation-not-found',
            'That recovery is no longer pending.',
          );
        const path =
          profile === firstRunFixture.invited.profile
            ? firstRunFixture.invited
            : firstRunFixture.own;
        ensureFirstRunAccount(profile, targetAlias, path.username);
        pending.delete(profile);
        return { applied: true };
      },
    ),
    list_account_devices: mockCommand(
      ({ accountStoreId }: { accountStoreId: string }) =>
        (deviceRows.get(accountStoreId) ?? []).map((entry) => ({ ...entry })),
    ),
    remove_account_device: mockCommand(
      async ({
        accountStoreId,
        deviceId,
      }: {
        accountStoreId: string;
        deviceId: string;
      }) => {
        const rows = deviceRows.get(accountStoreId) ?? [];
        const index = rows.findIndex((entry) => entry.id === deviceId);
        if (index >= 0) rows.splice(index, 1);
        return { deviceId, userChainSequence: 14, alreadyAbsent: index < 0 };
      },
    ),
    list_backup_enrollments: mockCommand(
      ({ accountStoreId }: { accountStoreId: string }) =>
        (backupRows.get(accountStoreId) ?? []).map((entry) => ({ ...entry })),
    ),
    // The fixture phrase derives the personal account's one backup key, and
    // a copy that keeps its first word and number is a near miss of it.
    verify_recovery_phrase: mockCommand(
      async ({
        accountStoreId,
        phrase,
      }: {
        accountStoreId: string;
        phrase: string;
      }) => {
        const store = accountStore(accountStoreId);
        if (!store)
          throw failure('store-not-found', 'That account is not available.');
        const typed = phrase.trim().split(/\s+/);
        const fixture = firstRunFixture.recoveryPhrase.split(' ');
        if (typed.length !== 17)
          throw failure(
            'invalid-request',
            `A recovery phrase has 17 words and numbers; this one has ${typed.length}.`,
          );
        const row = (backupRows.get(accountStoreId) ?? []).find(
          (entry) => entry.backupId === FIXTURE_BACKUP_ID,
        );
        const name = `${fixture[0]} ${fixture[1]}`;
        if (!row || typed[0] !== fixture[0] || typed[1] !== fixture[1])
          return {
            matchedBackupId: null,
            matchedName: null,
            revoked: false,
            backupAlias: null,
          };
        return {
          matchedBackupId:
            typed.join(' ') === fixture.join(' ') ? FIXTURE_BACKUP_ID : null,
          matchedName: name,
          revoked: false,
          backupAlias: row.backupAlias,
        };
      },
    ),
    revoke_owner_backup: mockCommand(
      async ({
        accountStoreId,
        backupAlias,
        backupId,
        confirmation,
      }: {
        accountStoreId: string;
        backupAlias: string;
        backupId: string;
        confirmation: string;
      }) => {
        if (confirmation !== backupAlias)
          throw failure(
            'invalid-request',
            'Type the exact backup alias to revoke it.',
          );
        const store = accountStore(accountStoreId);
        if (!store)
          throw failure('store-not-found', 'That account is not available.');
        const rows = backupRows.get(accountStoreId) ?? [];
        const index = rows.findIndex(
          (entry) => entry.backupAlias === backupAlias,
        );
        if (index >= 0 && rows[index]?.backupId !== backupId)
          throw failure(
            'invalid-request',
            'The backup configuration has changed. Refresh and try again.',
          );
        if (index >= 0) rows.splice(index, 1);
        return {
          backupAlias,
          accountAlias: store.account,
          backupId,
          userChainSequence: 15,
          alreadyAbsent: index < 0,
          removedLocalEnrollment: true,
        };
      },
    ),
    start_device_pairing: mockCommand(
      async ({ accountStoreId }: { accountStoreId: string }) => {
        const store = accountStore(accountStoreId);
        if (!store)
          throw failure('store-not-found', 'That account is not available.');
        const phrase = 'cobalt window';
        pairingOffers.set(accountStoreId, phrase);
        return { accountAlias: store.account, phrase };
      },
    ),
    resume_device_pairing_offer: mockCommand(
      async ({ accountStoreId }: { accountStoreId: string }) => {
        const store = accountStore(accountStoreId);
        const phrase = pairingOffers.get(accountStoreId);
        if (!store || !phrase)
          throw failure(
            'pending-operation-not-found',
            'This pairing request has expired or already finished.',
          );
        return { accountAlias: store.account, phrase };
      },
    ),
    finish_device_pairing: mockCommand(
      async ({ accountStoreId }: { accountStoreId: string }) => {
        const store = accountStore(accountStoreId);
        if (!store || !pairingOffers.has(accountStoreId))
          throw failure(
            'pending-operation-not-found',
            'Device pairing has not been completed yet.',
          );
        pairingOffers.delete(accountStoreId);
        return {
          alias: store.account,
          deviceId: `04${'5'.repeat(64)}`,
          userChainSequence: 15,
        };
      },
    ),
    accept_device_pairing: mockCommand(
      async ({
        profile,
        targetAlias,
      }: {
        profile: string;
        targetAlias: string;
      }) => {
        pairingAcceptances.add(`${profile}:${targetAlias}`);
        return {
          alias: targetAlias,
          deviceId: `04${'6'.repeat(64)}`,
          userChainSequence: 15,
        };
      },
    ),
    accept_go_profile_pairing: mockCommand(
      async ({
        request: { profile, targetAlias },
      }: {
        request: { profile: string; targetAlias: string };
      }) => {
        pairingAcceptances.add(`${profile}:${targetAlias}`);
        return {
          alias: targetAlias,
          deviceId: `04${'6'.repeat(64)}`,
          userChainSequence: 15,
        };
      },
    ),
    resume_device_pairing_acceptance: mockCommand(
      async ({
        profile,
        targetAlias,
      }: {
        profile: string;
        targetAlias: string;
      }) => {
        if (!pairingAcceptances.has(`${profile}:${targetAlias}`))
          throw failure(
            'pending-operation-not-found',
            'That pairing acceptance is no longer pending.',
          );
        pairingAcceptances.delete(`${profile}:${targetAlias}`);
        return {
          alias: targetAlias,
          deviceId: `04${'6'.repeat(64)}`,
          userChainSequence: 15,
        };
      },
    ),
    resume_go_profile_pairing: mockCommand(
      async ({
        profile,
        targetAlias,
      }: {
        profile: string;
        targetAlias: string;
      }) => {
        if (!pairingAcceptances.has(`${profile}:${targetAlias}`))
          throw failure(
            'pending-operation-not-found',
            'That pairing acceptance is no longer pending.',
          );
        pairingAcceptances.delete(`${profile}:${targetAlias}`);
        return {
          alias: targetAlias,
          deviceId: `04${'6'.repeat(64)}`,
          userChainSequence: 15,
        };
      },
    ),
    copy_go_profile_device: mockCommand(
      ({ targetAlias }: { targetAlias: string }) => ({
        alias: targetAlias,
        deviceId: `04${'7'.repeat(64)}`,
        userChainSequence: 15,
      }),
    ),
    set_account_passphrase: () => ({
      generation: 1,
      stretchVersion: 'v1',
      verified: true,
    }),
    change_account_passphrase: () => ({
      generation: 2,
      stretchVersion: 'v1',
      verified: true,
    }),
    verify_account_passphrase: () => ({
      generation: 2,
      stretchVersion: 'v1',
      verified: true,
    }),
    account_passphrase_status: () => ({
      configured: true,
      generation: 1,
    }),
    describe_reset: mockCommand(async ({ profile }: { profile: string }) => {
      const token = `reset-${profile}-${Date.now()}`;
      resetTokens.add(token);
      return {
        profile,
        resumables:
          profile === 'personal'
            ? [{ kind: 'team-creation' as const, alias: 'homelab' }]
            : [],
        artifacts: [{ kind: 'catalog-cache', entries: 5, bytes: 8192 }],
        token,
        expiresInSeconds: 60,
      };
    }),
    reset_server: mockCommand(
      async ({
        profileName,
        confirmedProfileName,
        token,
      }: {
        profileName: string;
        confirmedProfileName: string;
        token: string;
      }) => {
        if (profileName !== confirmedProfileName || !resetTokens.delete(token))
          throw failure(
            'invalid-request',
            'Review the reset details and enter the exact profile name to confirm.',
          );
        return { applied: true };
      },
    ),
  };
}

export function mockEnrollmentCommands(): MockDomain<
  typeof enrollmentCommands,
  (typeof ENROLLMENT_HOST_MEMBERS)[number]
> {
  return {
    // Optional methods only the native host offers; callers fall back to
    // other paths when a bridge lacks them.
    runFirstRunAccountOperation: undefined,
    firstRunOperationStatus: undefined,
  };
}
