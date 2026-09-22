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
  decodePassphraseStatus,
  decodePendingOperations,
  decodeResetPreview,
} from './enrollment';
import { checked, checkedMutation } from './transport';
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
  | 'accountPassphraseStatus'
  | 'describeReset'
  | 'resetServer'
> = {
  runFirstRunAccountOperation: (request) =>
    checkedMutation('run_first_run_account_operation', { request }, (value) => {
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
    checkedMutation(
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
    checkedMutation(
      'resume_first_run_account',
      { profile, alias },
      decodeMutation,
    ),
  setFirstRunPassphrase: ({ profile, alias, passphrase, confirmation }) =>
    checkedMutation(
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
    checkedMutation(
      'commit_owner_backup',
      { profile, accountAlias, backupAlias, phrase },
      decodeMutation,
    ),
  recoverOwnerAccount: (profile, targetAlias, phrase, deviceName) =>
    checkedMutation(
      'recover_owner_account',
      { profile, targetAlias, phrase, deviceName },
      decodeMutation,
    ),
  resumeOwnerRecovery: (profile, targetAlias, phrase, deviceName) =>
    checkedMutation(
      'resume_owner_recovery',
      { profile, targetAlias, phrase, deviceName },
      decodeMutation,
    ),
  listAccountDevices: (accountStoreId) =>
    checked('list_account_devices', { accountStoreId }, decodeAccountDevices),
  removeAccountDevice: (accountStoreId, deviceId) =>
    checkedMutation(
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
    checkedMutation(
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
    checkedMutation(
      'start_device_pairing',
      { accountStoreId },
      decodePairingOffer,
    ),
  resumeDevicePairingOffer: (accountStoreId) =>
    checkedMutation(
      'resume_device_pairing_offer',
      { accountStoreId },
      decodePairingOffer,
    ),
  finishDevicePairing: (accountStoreId) =>
    checkedMutation(
      'finish_device_pairing',
      { accountStoreId },
      decodeDeviceProvision,
    ),
  acceptDevicePairing: (profile, targetAlias, deviceName, phrase) =>
    checkedMutation(
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
    checkedMutation(
      'accept_go_profile_pairing',
      { request: { candidateId, profile, targetAlias, deviceName, phrase } },
      decodeDeviceProvision,
    ),
  resumeDevicePairingAcceptance: (profile, targetAlias) =>
    checkedMutation(
      'resume_device_pairing_acceptance',
      { profile, targetAlias },
      decodeDeviceProvision,
    ),
  resumeGoProfilePairing: (candidateId, profile, targetAlias) =>
    checkedMutation(
      'resume_go_profile_pairing',
      { candidateId, profile, targetAlias },
      decodeDeviceProvision,
    ),
  copyGoProfileDevice: (candidateId, profile, targetAlias) =>
    checkedMutation(
      'copy_go_profile_device',
      { candidateId, profile, targetAlias },
      decodeDeviceProvision,
    ),
  setAccountPassphrase: (accountStoreId, passphrase, confirmation) =>
    checkedMutation(
      'set_account_passphrase',
      { accountStoreId, passphrase, confirmation },
      decodePassphraseReport,
    ),
  changeAccountPassphrase: (
    accountStoreId,
    current,
    passphrase,
    confirmation,
  ) =>
    checkedMutation(
      'change_account_passphrase',
      { accountStoreId, current, passphrase, confirmation },
      decodePassphraseReport,
    ),
  verifyAccountPassphrase: (accountStoreId, passphrase) =>
    checked(
      'verify_account_passphrase',
      { accountStoreId, passphrase },
      decodePassphraseReport,
    ),
  accountPassphraseStatus: (accountStoreId) =>
    checked(
      'account_passphrase_status',
      { accountStoreId },
      decodePassphraseStatus,
    ),
  describeReset: (profile) =>
    checked('describe_reset', { profile }, decodeResetPreview),
  resetServer: (profileName, confirmedProfileName, token) =>
    checkedMutation(
      'reset_server',
      { profileName, confirmedProfileName, token },
      decodeMutation,
    ),
};
