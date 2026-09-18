import type { Bridge } from './contract';
import {
  decodeAccountDevices,
  decodeBackupEnrollments,
  decodeBackupPhrase,
  decodeBackupRevocation,
  decodeDeviceProvision,
  decodeDeviceRemoval,
  decodeGoProfileDiscovery,
  decodePairingOffer,
  decodePassphraseReport,
  decodePendingOperations,
  decodeResetPreview,
} from './enrollment';
import { checked } from './transport';
import { record } from './validation';
import { decodeMutation } from './vault-catalog';

export const enrollmentCommands: Pick<
  Bridge,
  | 'runFirstRunAccountOperation'
  | 'firstRunOperationStatus'
  | 'discoverGoProfiles'
  | 'listPendingOperations'
  | 'createFirstRunAccount'
  | 'resumeFirstRunAccount'
  | 'setFirstRunPassphrase'
  | 'prepareOwnerBackup'
  | 'commitOwnerBackup'
  | 'recoverOwnerAccount'
  | 'resumeOwnerRecovery'
  | 'listAccountDevices'
  | 'removeAccountDevice'
  | 'listBackupEnrollments'
  | 'revokeOwnerBackup'
  | 'startDevicePairing'
  | 'resumeDevicePairingOffer'
  | 'finishDevicePairing'
  | 'acceptDevicePairing'
  | 'acceptGoProfilePairing'
  | 'resumeDevicePairingAcceptance'
  | 'resumeGoProfilePairing'
  | 'copyGoProfileDevice'
  | 'setAccountPassphrase'
  | 'changeAccountPassphrase'
  | 'verifyAccountPassphrase'
  | 'describeReset'
  | 'resetServer'
> = {
  runFirstRunAccountOperation: (request) =>
    checked('run_first_run_account_operation', { request }, (value) => {
      const result = decodeMutation(value);
      if (!result.applied)
        throw new Error(
          'Account setup did not acknowledge completion. Check its status.',
        );
      return result;
    }),
  firstRunOperationStatus: (attempt) =>
    checked('first_run_operation_status', { attempt }, (value) => {
      const result = record(value, 'first_run_operation_status');
      const binding = record(
        result.attempt,
        'first_run_operation_status.attempt',
      );
      if (
        Object.entries(attempt).some(
          ([key, value]) => binding[key] !== value,
        ) ||
        !['complete', 'rejected', 'unknown', 'running'].includes(
          String(result.outcome),
        )
      )
        throw new Error(
          'Account setup status belongs to a different operation.',
        );
      return result.outcome as 'complete' | 'rejected' | 'unknown' | 'running';
    }),
  discoverGoProfiles: () =>
    checked('discover_go_profiles', undefined, decodeGoProfileDiscovery),
  listPendingOperations: (profile) =>
    checked('list_pending_operations', { profile }, decodePendingOperations),
  createFirstRunAccount: ({
    profile,
    alias,
    username,
    deviceName,
    email,
    invite,
    passphrase,
    passphraseConfirmation,
  }) =>
    checked(
      'create_first_run_account',
      {
        profile,
        alias,
        username,
        deviceName,
        email,
        invite,
        passphrase,
        passphraseConfirmation,
      },
      decodeMutation,
    ),
  resumeFirstRunAccount: (profile, alias) =>
    checked('resume_first_run_account', { profile, alias }, decodeMutation),
  setFirstRunPassphrase: ({ profile, alias, passphrase, confirmation }) =>
    checked(
      'set_first_run_passphrase',
      { profile, alias, passphrase, confirmation },
      decodeMutation,
    ),
  prepareOwnerBackup: (profile, accountAlias, backupAlias) =>
    checked(
      'prepare_owner_backup',
      { profile, accountAlias, backupAlias },
      decodeBackupPhrase,
    ),
  commitOwnerBackup: (profile, accountAlias, backupAlias, phrase) =>
    checked(
      'commit_owner_backup',
      { profile, accountAlias, backupAlias, phrase },
      decodeMutation,
    ),
  recoverOwnerAccount: (profile, targetAlias, phrase, deviceName) =>
    checked(
      'recover_owner_account',
      { profile, targetAlias, phrase, deviceName },
      decodeMutation,
    ),
  resumeOwnerRecovery: (profile, targetAlias, phrase, deviceName) =>
    checked(
      'resume_owner_recovery',
      { profile, targetAlias, phrase, deviceName },
      decodeMutation,
    ),
  listAccountDevices: (accountStoreId) =>
    checked('list_account_devices', { accountStoreId }, decodeAccountDevices),
  removeAccountDevice: (accountStoreId, deviceId) =>
    checked(
      'remove_account_device',
      { accountStoreId, deviceId },
      decodeDeviceRemoval,
    ),
  listBackupEnrollments: (accountStoreId) =>
    checked(
      'list_backup_enrollments',
      { accountStoreId },
      decodeBackupEnrollments,
    ),
  revokeOwnerBackup: (accountStoreId, backup, confirmation) =>
    checked(
      'revoke_owner_backup',
      {
        accountStoreId,
        backupAlias: backup.backupAlias,
        backupId: backup.backupId,
        confirmation,
      },
      (value) => {
        const revoked = decodeBackupRevocation(value);
        if (
          revoked.backupAlias !== backup.backupAlias ||
          revoked.accountAlias !== backup.accountAlias ||
          revoked.backupId !== backup.backupId
        )
          throw new Error(
            'revoke_owner_backup returned a different backup enrollment.',
          );
        return revoked;
      },
    ),
  startDevicePairing: (accountStoreId) =>
    checked('start_device_pairing', { accountStoreId }, decodePairingOffer),
  resumeDevicePairingOffer: (accountStoreId) =>
    checked(
      'resume_device_pairing_offer',
      { accountStoreId },
      decodePairingOffer,
    ),
  finishDevicePairing: (accountStoreId) =>
    checked('finish_device_pairing', { accountStoreId }, decodeDeviceProvision),
  acceptDevicePairing: (profile, targetAlias, deviceName, phrase) =>
    checked(
      'accept_device_pairing',
      { profile, targetAlias, deviceName, phrase },
      decodeDeviceProvision,
    ),
  acceptGoProfilePairing: (
    candidateId,
    profile,
    targetAlias,
    deviceName,
    phrase,
  ) =>
    checked(
      'accept_go_profile_pairing',
      { request: { candidateId, profile, targetAlias, deviceName, phrase } },
      decodeDeviceProvision,
    ),
  resumeDevicePairingAcceptance: (profile, targetAlias) =>
    checked(
      'resume_device_pairing_acceptance',
      { profile, targetAlias },
      decodeDeviceProvision,
    ),
  resumeGoProfilePairing: (candidateId, profile, targetAlias) =>
    checked(
      'resume_go_profile_pairing',
      { candidateId, profile, targetAlias },
      decodeDeviceProvision,
    ),
  copyGoProfileDevice: (candidateId, profile, targetAlias) =>
    checked(
      'copy_go_profile_device',
      { candidateId, profile, targetAlias },
      decodeDeviceProvision,
    ),
  setAccountPassphrase: (accountStoreId, passphrase, confirmation) =>
    checked(
      'set_account_passphrase',
      { accountStoreId, passphrase, confirmation },
      decodePassphraseReport,
    ),
  changeAccountPassphrase: (accountStoreId, passphrase, confirmation) =>
    checked(
      'change_account_passphrase',
      { accountStoreId, passphrase, confirmation },
      decodePassphraseReport,
    ),
  verifyAccountPassphrase: (accountStoreId, passphrase) =>
    checked(
      'verify_account_passphrase',
      { accountStoreId, passphrase },
      decodePassphraseReport,
    ),
  describeReset: (profile) =>
    checked('describe_reset', { profile }, decodeResetPreview),
  resetServer: (profile, confirmation, token) =>
    checked('reset_server', { profile, confirmation, token }, decodeMutation),
};
