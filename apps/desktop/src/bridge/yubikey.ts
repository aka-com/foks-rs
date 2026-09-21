import type { StoreRef } from '../model';
import { decodePassphraseReport } from './enrollment';
import {
  array,
  bool,
  deviceMemberId,
  entityId,
  integer,
  nullable,
  record,
  string,
} from './validation';

export interface YubiEnrollment {
  alias: string;
  state: 'pending' | 'complete';
  /** The same authenticated key id carried by AccountDevice.id. */
  deviceId?: string;
  cardSerial?: number;
}

export interface CreateYubiAccountRequest {
  profile: string;
  alias: string;
  username: string;
  deviceName: string;
  email: string;
  invite: string;
  passphrase?: string;
  passphraseConfirmation?: string;
  cardSerial: number;
  signingSlot: number;
  pqSlot: number;
  pin: string;
  puk: string;
  pinAttempts: number;
  pukAttempts: number;
}

export interface ProvisionYubiDeviceRequest {
  accountStoreId: StoreRef;
  targetAlias: string;
  deviceName: string;
  cardSerial: number;
  signingSlot: number;
  pqSlot: number;
  pin: string;
  puk: string;
  pinAttempts: number;
  pukAttempts: number;
}

export type YubiCommand =
  | { command: 'create_yubi_account'; args: CreateYubiAccountRequest }
  | {
      command: 'resume_yubi_account';
      args: { profile: string; alias: string; pin: string };
    }
  | { command: 'provision_yubi_device'; args: ProvisionYubiDeviceRequest }
  | {
      command: 'sync_yubi_account';
      args: {
        profile: string;
        alias: string;
        pin: string;
        withFederation: boolean;
      };
    }
  | { command: 'yubi_pin_status'; args: { profile: string; alias: string } }
  | {
      command: 'change_yubi_pin';
      args: { profile: string; alias: string; oldPin: string; newPin: string };
    }
  | {
      command: 'set_yubi_passphrase' | 'change_yubi_passphrase';
      args: {
        profile: string;
        alias: string;
        pin: string;
        passphrase: string;
        confirmation: string;
      };
    }
  | {
      command: 'verify_yubi_passphrase';
      args: { profile: string; alias: string; pin: string; passphrase: string };
    }
  | {
      command: 'change_yubi_puk';
      args: { profile: string; alias: string; oldPuk: string; newPuk: string };
    }
  | {
      command: 'unblock_yubi_pin';
      args: { profile: string; alias: string; puk: string; newPin: string };
    }
  | {
      command: 'rotate_yubi_management_key';
      args: { profile: string; alias: string; pin: string };
    }
  | {
      command: 'resume_yubi_management_key';
      args: { profile: string; alias: string; pin?: string };
    }
  | {
      command: 'recover_yubi_management_key';
      args: { accountStoreId: StoreRef; yubiAlias: string };
    }
  | {
      command: 'recover_yubi_subkey';
      args: { profile: string; alias: string; pin: string };
    }
  | {
      command: 'revoke_yubi_device';
      args: {
        accountStoreId: StoreRef;
        yubiAlias: string;
        confirmation: string;
      };
    };

export const decodeYubiCards = (value: unknown): { serial: number }[] =>
  array(value, 'list_yubi_cards response', (entry, at) => ({
    serial: integer(record(entry, at).serial, `${at}.serial`),
  }));
export const decodeYubiAccounts = (value: unknown): YubiEnrollment[] =>
  array(value, 'list_yubi_accounts response', (entry, at) => {
    const item = record(entry, at);
    const state = string(item.state, `${at}.state`);
    if (state !== 'pending' && state !== 'complete')
      throw new Error(`${at}.state is invalid`);
    const deviceId =
      item.deviceId == null
        ? undefined
        : deviceMemberId(item.deviceId, `${at}.deviceId`);
    if (deviceId && !deviceId.startsWith('08'))
      throw new Error(`${at}.deviceId is not a card key`);
    const cardSerial =
      item.cardSerial == null
        ? undefined
        : integer(item.cardSerial, `${at}.cardSerial`);
    if (cardSerial !== undefined && cardSerial <= 0)
      throw new Error(`${at}.cardSerial is invalid`);
    return {
      alias: string(item.alias, `${at}.alias`),
      state,
      ...(deviceId ? { deviceId } : {}),
      ...(cardSerial ? { cardSerial } : {}),
    };
  });

export function decodeYubiResult(
  command: YubiCommand['command'],
  value: unknown,
): Record<string, unknown> {
  const item = record(value, `${command} response`);
  if (
    command === 'yubi_pin_status' ||
    command === 'change_yubi_pin' ||
    command === 'unblock_yubi_pin'
  ) {
    return {
      remaining: integer(item.remaining, `${command}.remaining`),
      blocked: bool(item.blocked, `${command}.blocked`),
    };
  } else if (command.includes('passphrase')) {
    return { ...decodePassphraseReport(item) };
  } else if (
    command === 'create_yubi_account' ||
    command === 'resume_yubi_account' ||
    command === 'provision_yubi_device'
  ) {
    const yubiId = string(item.yubiId, `${command}.yubiId`);
    if (!/^08[0-9a-f]{66}$/.test(yubiId))
      throw new Error(`${command}.yubiId must be a canonical hardware key id`);
    const subkeyId = entityId(item.subkeyId, '0d', `${command}.subkeyId`);
    return {
      alias: string(item.alias, `${command}.alias`),
      username: string(item.username, `${command}.username`),
      yubiId,
      subkeyId,
      userChainSequence: integer(
        item.userChainSequence,
        `${command}.userChainSequence`,
      ),
      managementEnrolled: bool(
        item.managementEnrolled,
        `${command}.managementEnrolled`,
      ),
    };
  } else if (command === 'sync_yubi_account') {
    return {
      username: string(item.username, `${command}.username`),
      userChainSequence: integer(
        item.userChainSequence,
        `${command}.userChainSequence`,
      ),
      directories: integer(item.directories, `${command}.directories`),
      entries: integer(item.entries, `${command}.entries`),
      federation: array(
        item.federation,
        `${command}.federation`,
        (value, at) => {
          const row = record(value, at);
          return {
            localProfile: string(row.localProfile, `${at}.localProfile`),
            localTeamAlias: string(row.localTeamAlias, `${at}.localTeamAlias`),
            refreshed: bool(row.refreshed, `${at}.refreshed`),
            deferred: nullable(row.deferred, `${at}.deferred`, string),
          };
        },
      ),
    };
  } else if (command === 'change_yubi_puk') {
    if (item.changed !== true)
      throw new Error(`${command}.changed must be true`);
    return { alias: string(item.alias, `${command}.alias`), changed: true };
  } else if (command === 'recover_yubi_subkey') {
    const subkeyId = entityId(item.subkeyId, '0d', `${command}.subkeyId`);
    return {
      alias: string(item.alias, `${command}.alias`),
      subkeyId,
      certificateCount: integer(
        item.certificateCount,
        `${command}.certificateCount`,
      ),
    };
  } else if (command === 'revoke_yubi_device') {
    return {
      alias: string(item.alias, `${command}.alias`),
      userChainSequence: integer(
        item.userChainSequence,
        `${command}.userChainSequence`,
      ),
      removedLocalCredential: bool(
        item.removedLocalCredential,
        `${command}.removedLocalCredential`,
      ),
    };
  }
  return {
    alias: string(item.alias, `${command}.alias`),
    managementEnrolled: bool(
      item.managementEnrolled,
      `${command}.managementEnrolled`,
    ),
    managementGeneration: nullable(
      item.managementGeneration,
      `${command}.managementGeneration`,
      integer,
    ),
  };
}
