import type { CheckedProfileResponse } from './servers';
import {
  array,
  bool,
  deviceMemberId,
  entityId,
  integer,
  optionalString,
  record,
  string,
} from './validation';

export type PendingOperationKind =
  | 'account-signup'
  | 'device-provision'
  | 'pairing-offer'
  | 'pairing-acceptance'
  | 'account-recovery'
  | 'yubi-enrollment'
  | 'team-creation'
  | 'team-member-addition'
  | 'team-member-edit'
  | 'federation-expulsion'
  | 'team-rekey';

export interface PendingOperation {
  kind: PendingOperationKind;
  alias: string;
  target?: string;
}

export interface FirstRunAccountRequest {
  profile: string;
  alias: string;
  username: string;
  deviceName: string;
  email: string;
  invite: string;
  passphrase?: string;
  passphraseConfirmation?: string;
}

export interface TrackedAccountAttempt {
  id: string;
  profile: string;
  hostId: string;
  alias: string;
  deviceName: string;
  candidateId?: string;
  kind: 'signup' | 'recovery' | 'copy' | 'pairing';
}

export interface TrackedAccountRequest {
  attempt: TrackedAccountAttempt;
  resume: boolean;
  deviceName: string;
  username?: string;
  email?: string;
  invite?: string;
  phrase?: string;
  candidateId?: string;
}

export interface FirstRunPassphraseRequest {
  profile: string;
  alias: string;
  passphrase: string;
  confirmation: string;
}

export interface BackupPhraseResponse {
  backupAlias: string;
  /** Ephemeral recovery phrase; should not be cached or persisted. */
  phrase: string;
}

export interface GoProfileCandidate {
  candidateId: string;
  username?: string;
  serverHint?: string;
  hostId: string;
  userId: string;
  deviceId: string;
  role: string;
  storageKind:
    | 'plaintext'
    | 'passphrase'
    | 'macos-keychain'
    | 'noise-file'
    | 'generic-keychain';
  hidden: boolean;
  provisional: boolean;
  pairable: boolean;
  copyable: boolean;
}

export interface GoProfileDiscovery {
  installed: boolean;
  candidates: GoProfileCandidate[];
}

export interface AccountDevice {
  id: string;
  /** Authenticated device name reported by the agent, if available. */
  name?: string;
  role: 'owner' | 'admin' | 'member';
  current: boolean;
}

export interface BackupEnrollment {
  backupAlias: string;
  accountAlias: string;
  backupId: string;
}

export interface BackupRevocation extends BackupEnrollment {
  userChainSequence: number;
  alreadyAbsent: boolean;
  removedLocalEnrollment: true;
}

export interface PairingOffer {
  accountAlias: string;
  /** Ephemeral pairing phrase; not persisted. */
  phrase: string;
}

export interface DeviceProvision {
  alias: string;
  deviceId: string;
  userChainSequence: number;
}

export interface PassphraseReport {
  generation: number;
  stretchVersion: 'v1';
  verified: true;
}

export interface ResetPreview {
  profile: string;
  resumables: PendingOperation[];
  artifacts: { kind: string; entries: number; bytes: number }[];
  /** Single-use confirmation token valid only for this reset preview. */
  token: string;
  expiresInSeconds: number;
}

/** Mock fixtures used for local development and testing. */
export interface FirstRunFixturePath {
  profile: string;
  accountAlias: string;
  server: string;
  typo: string;
  username: string;
  deviceName: string;
  admin?: string;
  groupName?: string;
  groupAlias?: string;
  groupTeamIdHex?: string;
  report: CheckedProfileResponse;
}

export interface FirstRunFixture {
  invited: FirstRunFixturePath;
  own: FirstRunFixturePath;
  backupPhrase: string;
}

const PENDING_KINDS: readonly PendingOperationKind[] = [
  'account-signup',
  'device-provision',
  'pairing-offer',
  'pairing-acceptance',
  'account-recovery',
  'yubi-enrollment',
  'team-creation',
  'team-member-addition',
  'team-member-edit',
  'federation-expulsion',
  'team-rekey',
];

function decodePendingOperation(value: unknown, at: string): PendingOperation {
  const item = record(value, at);
  const kind = string(item.kind, `${at}.kind`);
  if (!(PENDING_KINDS as readonly string[]).includes(kind)) {
    throw new Error(`${at}.kind is not a pending-operation kind`);
  }
  return {
    kind: kind as PendingOperationKind,
    alias: string(item.alias, `${at}.alias`),
    target: optionalString(item.target, `${at}.target`),
  };
}

export const decodePendingOperations = (value: unknown): PendingOperation[] =>
  array(value, 'list_pending_operations response', decodePendingOperation);

export function decodeBackupPhrase(value: unknown): BackupPhraseResponse {
  const item = record(value, 'prepare_owner_backup response');
  return {
    backupAlias: string(item.backupAlias, 'prepare_owner_backup.backupAlias'),
    phrase: string(item.phrase, 'prepare_owner_backup.phrase'),
  };
}

function decodeGoProfileCandidate(
  value: unknown,
  at: string,
): GoProfileCandidate {
  const item = record(value, at);
  const candidateId = string(item.candidateId, `${at}.candidateId`);
  if (!/^[0-9a-f]{64}$/.test(candidateId))
    throw new Error(`${at}.candidateId is not canonical hex`);
  const username = optionalString(item.username, `${at}.username`);
  const serverHint = optionalString(item.serverHint, `${at}.serverHint`);
  const hostId = entityId(item.hostId, '02', `${at}.hostId`);
  const userId = entityId(item.userId, '01', `${at}.userId`);
  const deviceId = string(item.deviceId, `${at}.deviceId`);
  if (
    !/^(04|10|13)[0-9a-f]{64}$/.test(deviceId) &&
    !/^08[0-9a-f]{66}$/.test(deviceId)
  )
    throw new Error(`${at}.deviceId is not a supported FOKS key id`);
  const role = string(item.role, `${at}.role`);
  const member = /^member\((-?(?:0|[1-9][0-9]{0,4}))\)$/.exec(role);
  if (
    !['none', 'admin', 'owner'].includes(role) &&
    (!member || Math.abs(Number(member[1])) > 16384)
  )
    throw new Error(`${at}.role is invalid`);
  const storageKind = string(item.storageKind, `${at}.storageKind`);
  if (
    ![
      'plaintext',
      'passphrase',
      'macos-keychain',
      'noise-file',
      'generic-keychain',
    ].includes(storageKind)
  )
    throw new Error(`${at}.storageKind is unsupported`);
  const hidden = bool(item.hidden, `${at}.hidden`);
  const provisional = bool(item.provisional, `${at}.provisional`);
  const pairable = bool(item.pairable, `${at}.pairable`);
  const copyable = bool(item.copyable, `${at}.copyable`);
  if ((hidden || provisional) && (pairable || copyable))
    throw new Error(`${at} exposes an incomplete Go profile`);
  if (
    copyable &&
    (!pairable ||
      !deviceId.startsWith('04') ||
      storageKind !== 'macos-keychain')
  )
    throw new Error(`${at} exposes an unsupported device copy`);
  return {
    candidateId,
    ...(username ? { username } : {}),
    ...(serverHint ? { serverHint } : {}),
    hostId,
    userId,
    deviceId,
    role,
    storageKind: storageKind as GoProfileCandidate['storageKind'],
    hidden,
    provisional,
    pairable,
    copyable,
  };
}

export function decodeGoProfileDiscovery(value: unknown): GoProfileDiscovery {
  const item = record(value, 'discover_go_profiles response');
  const installed = bool(item.installed, 'discover_go_profiles.installed');
  const candidates = array(
    item.candidates,
    'discover_go_profiles.candidates',
    decodeGoProfileCandidate,
  );
  if (candidates.length > 128)
    throw new Error('discover_go_profiles returned too many candidates');
  if (!installed && candidates.length)
    throw new Error(
      'discover_go_profiles returned candidates without an installation',
    );
  if (
    new Set(candidates.map((candidate) => candidate.candidateId)).size !==
    candidates.length
  )
    throw new Error('discover_go_profiles returned duplicate candidates');
  return { installed, candidates };
}

function decodeAccountDevice(value: unknown, at: string): AccountDevice {
  const item = record(value, at);
  const role = string(item.role, `${at}.role`);
  if (role !== 'owner' && role !== 'admin' && role !== 'member')
    throw new Error(`${at}.role is invalid`);
  return {
    id: deviceMemberId(item.id, `${at}.id`),
    name: optionalString(item.name, `${at}.name`),
    role,
    current: bool(item.current, `${at}.current`),
  };
}
export const decodeAccountDevices = (value: unknown): AccountDevice[] =>
  array(value, 'list_account_devices response', decodeAccountDevice);

function decodeBackupEnrollment(value: unknown, at: string): BackupEnrollment {
  const item = record(value, at);
  return {
    backupAlias: string(item.backupAlias, `${at}.backupAlias`),
    accountAlias: string(item.accountAlias, `${at}.accountAlias`),
    backupId: entityId(item.backupId, '10', `${at}.backupId`),
  };
}
export const decodeBackupEnrollments = (value: unknown): BackupEnrollment[] =>
  array(value, 'list_backup_enrollments response', decodeBackupEnrollment);

export function decodeBackupRevocation(value: unknown): BackupRevocation {
  const item = record(value, 'revoke_owner_backup response');
  if (item.removedLocalEnrollment !== true)
    throw new Error('revoke_owner_backup.removedLocalEnrollment must be true');
  return {
    backupAlias: string(item.backupAlias, 'revoke_owner_backup.backupAlias'),
    accountAlias: string(item.accountAlias, 'revoke_owner_backup.accountAlias'),
    backupId: entityId(item.backupId, '10', 'revoke_owner_backup.backupId'),
    userChainSequence: integer(
      item.userChainSequence,
      'revoke_owner_backup.userChainSequence',
    ),
    alreadyAbsent: bool(
      item.alreadyAbsent,
      'revoke_owner_backup.alreadyAbsent',
    ),
    removedLocalEnrollment: true,
  };
}

export function decodePairingOffer(value: unknown): PairingOffer {
  const item = record(value, 'pairing offer response');
  return {
    accountAlias: string(item.accountAlias, 'pairing offer.accountAlias'),
    phrase: string(item.phrase, 'pairing offer.phrase'),
  };
}
export function decodeDeviceProvision(value: unknown): DeviceProvision {
  const item = record(value, 'device provision response');
  return {
    alias: string(item.alias, 'device provision.alias'),
    deviceId: entityId(item.deviceId, '04', 'device provision.deviceId'),
    userChainSequence: integer(
      item.userChainSequence,
      'device provision.userChainSequence',
    ),
  };
}

export function decodeDeviceRemoval(value: unknown): {
  deviceId: string;
  userChainSequence: number;
  alreadyAbsent: boolean;
} {
  const item = record(value, 'remove_account_device response');
  return {
    deviceId: entityId(item.deviceId, '04', 'remove_account_device.deviceId'),
    userChainSequence: integer(
      item.userChainSequence,
      'remove_account_device.userChainSequence',
    ),
    alreadyAbsent: bool(
      item.alreadyAbsent,
      'remove_account_device.alreadyAbsent',
    ),
  };
}
export function decodePassphraseReport(value: unknown): PassphraseReport {
  const item = record(value, 'passphrase response');
  if (item.stretchVersion !== 'v1' || item.verified !== true)
    throw new Error('passphrase response is invalid');
  return {
    generation: integer(item.generation, 'passphrase.generation'),
    stretchVersion: 'v1',
    verified: true,
  };
}

function decodeResetArtifact(
  value: unknown,
  at: string,
): ResetPreview['artifacts'][number] {
  const item = record(value, at);
  return {
    kind: string(item.kind, `${at}.kind`),
    entries: integer(item.entries, `${at}.entries`),
    bytes: integer(item.bytes, `${at}.bytes`),
  };
}
export function decodeResetPreview(value: unknown): ResetPreview {
  const item = record(value, 'describe_reset response');
  return {
    profile: string(item.profile, 'describe_reset.profile'),
    resumables: array(
      item.resumables,
      'describe_reset.resumables',
      decodePendingOperation,
    ),
    artifacts: array(
      item.artifacts,
      'describe_reset.artifacts',
      decodeResetArtifact,
    ),
    token: string(item.token, 'describe_reset.token'),
    expiresInSeconds: integer(
      item.expiresInSeconds,
      'describe_reset.expiresInSeconds',
    ),
  };
}
