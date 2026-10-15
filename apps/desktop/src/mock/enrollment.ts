/**
 * The mock's enrollment commands, the members of
 * `bridge/commands-enrollment.ts`: first-run account setup, recovery
 * phrases, devices, pairing, passphrases and profile reset. They answer
 * from the shared store records with decoded values.
 */

import type { enrollmentCommands } from '../bridge/commands-enrollment';
import { failure, type MockDomain, type MockState } from './state';

/** The personal account's one recovery phrase: the key the fixture phrase derives. */
const FIXTURE_BACKUP_ID = `10${'4'.repeat(64)}`;

export function mockEnrollmentCommands(
  state: MockState,
): MockDomain<typeof enrollmentCommands> {
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
            // Hardware security key device record (prefix `08`), providing
            // fixture data that distinguishes card-bound keys from host keys.
            {
              id: `08${'5'.repeat(64)}`,
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
    // Optional methods only the native host offers; callers fall back to
    // other paths when a bridge lacks them.
    runFirstRunAccountOperation: undefined,
    firstRunOperationStatus: undefined,
    discoverGoProfiles: async () => ({ installed: false, candidates: [] }),
    listPendingOperations: async (profile) =>
      (pending.get(profile) ?? []).map((operation) => ({ ...operation })),
    createFirstRunAccount: async ({ profile, alias, username }) => {
      pending.set(profile, [{ kind: 'account-signup', alias }]);
      ensureFirstRunAccount(profile, alias, username);
      pending.delete(profile);
      return { applied: true };
    },
    resumeFirstRunAccount: async (profile, alias) => {
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
    setFirstRunPassphrase: async () => ({ applied: true }),
    prepareOwnerBackup: async (_profile, _accountAlias, backupAlias) => ({
      backupAlias,
      phrase: firstRunFixture.recoveryPhrase,
    }),
    commitOwnerBackup: async (_profile, _accountAlias, backupAlias, phrase) => {
      if (
        backupAlias !== 'paper' ||
        phrase !== firstRunFixture.recoveryPhrase
      ) {
        throw failure('invalid-request', 'The recovery phrase does not match.');
      }
      return { applied: true };
    },
    recoverOwnerAccount: async (profile, targetAlias) => {
      pending.set(profile, [{ kind: 'account-recovery', alias: targetAlias }]);
      const path =
        profile === firstRunFixture.invited.profile
          ? firstRunFixture.invited
          : firstRunFixture.own;
      ensureFirstRunAccount(profile, targetAlias, path.username);
      pending.delete(profile);
      return { applied: true };
    },
    resumeOwnerRecovery: async (profile, targetAlias) => {
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
    listAccountDevices: async (accountStoreId) =>
      (deviceRows.get(accountStoreId) ?? []).map((entry) => ({ ...entry })),
    removeAccountDevice: async (accountStoreId, deviceId) => {
      const rows = deviceRows.get(accountStoreId) ?? [];
      const index = rows.findIndex((entry) => entry.id === deviceId);
      if (index >= 0) rows.splice(index, 1);
      return { deviceId, userChainSequence: 14, alreadyAbsent: index < 0 };
    },
    listBackupEnrollments: async (accountStoreId) =>
      (backupRows.get(accountStoreId) ?? []).map((entry) => ({ ...entry })),
    // The fixture phrase derives the personal account's one backup key, and
    // a copy that keeps its first word and number is a near miss of it.
    verifyRecoveryPhrase: async (accountStoreId, phrase) => {
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
    revokeOwnerBackup: async (accountStoreId, backup, confirmation) => {
      if (confirmation !== backup.backupAlias)
        throw failure(
          'invalid-request',
          'Type the exact backup alias to revoke it.',
        );
      const store = accountStore(accountStoreId);
      if (!store || store.account !== backup.accountAlias)
        throw failure('store-not-found', 'That account is not available.');
      const rows = backupRows.get(accountStoreId) ?? [];
      const index = rows.findIndex(
        (entry) => entry.backupAlias === backup.backupAlias,
      );
      if (index >= 0 && rows[index]?.backupId !== backup.backupId)
        throw failure(
          'invalid-request',
          'The backup configuration has changed. Refresh and try again.',
        );
      if (index >= 0) rows.splice(index, 1);
      return {
        ...backup,
        userChainSequence: 15,
        alreadyAbsent: index < 0,
        removedLocalEnrollment: true as const,
      };
    },
    startDevicePairing: async (accountStoreId) => {
      const store = accountStore(accountStoreId);
      if (!store)
        throw failure('store-not-found', 'That account is not available.');
      const phrase = 'cobalt window';
      pairingOffers.set(accountStoreId, phrase);
      return { accountAlias: store.account, phrase };
    },
    resumeDevicePairingOffer: async (accountStoreId) => {
      const store = accountStore(accountStoreId);
      const phrase = pairingOffers.get(accountStoreId);
      if (!store || !phrase)
        throw failure(
          'pending-operation-not-found',
          'This pairing request has expired or already finished.',
        );
      return { accountAlias: store.account, phrase };
    },
    finishDevicePairing: async (accountStoreId) => {
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
    acceptDevicePairing: async (profile, targetAlias) => {
      pairingAcceptances.add(`${profile}:${targetAlias}`);
      return {
        alias: targetAlias,
        deviceId: `04${'6'.repeat(64)}`,
        userChainSequence: 15,
      };
    },
    acceptGoProfilePairing: async (_candidateId, profile, targetAlias) => {
      pairingAcceptances.add(`${profile}:${targetAlias}`);
      return {
        alias: targetAlias,
        deviceId: `04${'6'.repeat(64)}`,
        userChainSequence: 15,
      };
    },
    resumeDevicePairingAcceptance: async (profile, targetAlias) => {
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
    resumeGoProfilePairing: async (_candidateId, profile, targetAlias) => {
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
    copyGoProfileDevice: async (_candidateId, _profile, targetAlias) => ({
      alias: targetAlias,
      deviceId: `04${'7'.repeat(64)}`,
      userChainSequence: 15,
    }),
    setAccountPassphrase: async () => ({
      generation: 1,
      stretchVersion: 'v1' as const,
      verified: true as const,
    }),
    changeAccountPassphrase: async () => ({
      generation: 2,
      stretchVersion: 'v1' as const,
      verified: true as const,
    }),
    verifyAccountPassphrase: async () => ({
      generation: 2,
      stretchVersion: 'v1' as const,
      verified: true as const,
    }),
    accountPassphraseStatus: async () => ({
      configured: true,
      generation: 1,
    }),
    describeReset: async (profile) => {
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
    },
    resetServer: async (profileName, confirmedProfileName, token) => {
      if (profileName !== confirmedProfileName || !resetTokens.delete(token))
        throw failure(
          'invalid-request',
          'Review the reset details and enter the exact profile name to confirm.',
        );
      return { applied: true };
    },
  };
}
