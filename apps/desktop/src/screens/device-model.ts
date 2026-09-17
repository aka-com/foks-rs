/**
 * The keys one account holds on this Mac, as one list.
 *
 * The agent answers with four unrelated lists, and they are not all about the
 * same thing: the authenticated devices and the local backup enrollments are
 * answered per account, while the card enrollments and the card in the port
 * are answered per server profile, for every account on it. Both Account and
 * Devices present them as one set of keys, so the reading and the row model
 * are here rather than written twice, and every row says which of the two it
 * came from.
 */

import { enqueueProfileWork } from '../bridge';
import type {
  AccountDevice,
  BackupEnrollment,
  Bridge,
  YubiEnrollment,
} from '../bridge';
import type { FoksIconName } from '../icons';
import type { StoreRef } from '../model';

/** What the two per-account and the two per-profile calls answered with. */
export interface DeviceLists {
  /** Per account: the devices this account is authenticated on. */
  devices: AccountDevice[];
  /** Per account: the paper-key enrollments this Mac holds for it. */
  backups: BackupEnrollment[];
  /** Per profile: the card enrollments on that server, on any account. */
  yubi: YubiEnrollment[];
  /** Per profile: the cards in this Mac's ports right now. */
  cards: { serial: number }[];
}

/** The empty set, for a page that has read nothing yet. */
export const NO_DEVICES: DeviceLists = {
  devices: [],
  backups: [],
  yubi: [],
  cards: [],
};

/**
 * Read one account's keys and its profile's card enrollments, in one turn of
 * that profile's work queue. The calls fail separately, and a caller decides
 * what to do with the failure; they are read together because both pages list
 * them together. Asking which card is in the port drives the reader, so a
 * caller that does not list the connected card asks for no card probe.
 */
export function readAccountAndProfileKeys(
  bridge: Bridge,
  profile: string,
  store: StoreRef,
  options: { cards?: boolean } = {},
): Promise<DeviceLists> {
  return enqueueProfileWork(bridge, profile, async () => {
    const devices = await bridge.listAccountDevices(store);
    const backups = await bridge.listBackupEnrollments(store);
    const cards =
      options.cards === false ? [] : await bridge.listYubiCards(profile);
    const yubi = await bridge.listYubiAccounts(profile);
    return { devices, backups, cards, yubi };
  });
}

/**
 * What a key is, as the row and the detail page name it. A card holds two
 * different objects, so they carry two different names: the device key an
 * account is authenticated with on a card is a "key on a card", while the
 * object `list_yubi_accounts` answers with, for the whole server profile, is
 * an "enrollment".
 */
export type DeviceKind =
  'Computer' | 'Paper key' | 'Key on a card' | 'Enrollment';

/** Which list a row came from, and the record it stands for. */
export type DeviceSource =
  | { kind: 'device'; device: AccountDevice }
  | { kind: 'backup'; backup: BackupEnrollment }
  | { kind: 'yubi'; entry: YubiEnrollment };

/** One key on the account: what the agent says about it, and nothing else. */
export interface DeviceEntry {
  /** The `device=` parameter this row's own page is addressed by. */
  address: string;
  name: string;
  kind: DeviceKind;
  /** The bare role, capitalized, where the agent reports one. */
  role?: string;
  /** The full key id, where the agent reports one. */
  keyId?: string;
  /** What the id is called on the detail page. */
  keyLabel: string;
  icon: FoksIconName;
  /** Whether this is the key this Mac is authenticated with right now. */
  current: boolean;
  /**
   * Who the agent answered for: `account` for a row it listed under this
   * account, `profile` for one it listed for the server, on any account.
   */
  scope: 'account' | 'profile';
  source: DeviceSource;
}

/**
 * An authenticated device's id says what kind of key it is: `08` is a key on a
 * card, and everything else is a computer holding its own key.
 */
export function deviceIsCard(device: AccountDevice): boolean {
  return device.id.startsWith('08');
}

/** "owner" as a row reads it: the bare role, capitalized. */
export function roleLabel(role: AccountDevice['role']): string {
  return role.charAt(0).toUpperCase() + role.slice(1);
}

/** The name a device row carries when the agent reports none. */
export function deviceName(device: AccountDevice): string {
  return device.name ?? (deviceIsCard(device) ? 'YubiKey' : 'Device');
}

function deviceEntry(device: AccountDevice): DeviceEntry {
  const card = deviceIsCard(device);
  return {
    address: device.id,
    name: deviceName(device),
    kind: card ? 'Key on a card' : 'Computer',
    role: roleLabel(device.role),
    keyId: device.id,
    keyLabel: 'Device key',
    icon: card ? 'key' : 'laptop',
    current: device.current,
    scope: 'account',
    source: { kind: 'device', device },
  };
}

function backupEntry(backup: BackupEnrollment): DeviceEntry {
  return {
    address: backup.backupId,
    name: backup.backupAlias,
    kind: 'Paper key',
    keyId: backup.backupId,
    keyLabel: 'Backup id',
    icon: 'file',
    current: false,
    scope: 'account',
    source: { kind: 'backup', backup },
  };
}

/**
 * An enrollment belongs to the server profile. Once its key is prepared, its
 * device id matches the authenticated device list; early preparations have none.
 */
function yubiEntry(entry: YubiEnrollment): DeviceEntry {
  return {
    address: `yubi:${entry.alias}`,
    name: entry.alias,
    kind: 'Enrollment',
    keyId: entry.deviceId,
    keyLabel: 'Device key',
    icon: 'key',
    current: false,
    scope: 'profile',
    source: { kind: 'yubi', entry },
  };
}

/** Every key on the account, in the order the page lists them. */
export function deviceEntries(lists: DeviceLists): DeviceEntry[] {
  return [
    ...lists.devices.map(deviceEntry),
    ...lists.backups.map(backupEntry),
    ...lists.yubi.map(yubiEntry),
  ];
}

/** The key a `device=` address names, or nothing when this Mac has lost it. */
export function deviceAt(
  lists: DeviceLists,
  address: string,
): DeviceEntry | undefined {
  return deviceEntries(lists).find((entry) => entry.address === address);
}

/** Select only an exact, unambiguous native identity; list order is not identity. */
export function enrollmentForCard(
  enrollments: readonly YubiEnrollment[],
  serial: number,
): YubiEnrollment | undefined {
  const matches = enrollments.filter((entry) => entry.cardSerial === serial);
  return matches.length === 1 ? matches[0] : undefined;
}
export function enrollmentForDevice(
  enrollments: readonly YubiEnrollment[],
  id: string,
): YubiEnrollment | undefined {
  const matches = enrollments.filter(
    (entry) => entry.deviceId === id && entry.state === 'complete',
  );
  return matches.length === 1 ? matches[0] : undefined;
}
