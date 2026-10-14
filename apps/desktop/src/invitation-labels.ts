import {
  readStorage,
  STORAGE_KEYS,
  writeStorage,
} from './platform/device-storage';

export type InvitationGrantRole = 'member' | 'admin' | 'owner';

export interface InvitationLabel {
  label: string;
  role: InvitationGrantRole;
  created: number;
}

type Stored = Record<string, InvitationLabel>;

function readAll(): Stored {
  try {
    const raw = readStorage(STORAGE_KEYS.invitationLabels);
    if (!raw) return {};
    const parsed: unknown = JSON.parse(raw);
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed)
      ? (parsed as Stored)
      : {};
  } catch {
    return {};
  }
}

function writeAll(value: Stored): void {
  writeStorage(STORAGE_KEYS.invitationLabels, JSON.stringify(value));
}

function key(profile: string, account: string, operationId: string): string {
  return `${profile}/${account}/${operationId}`;
}

export function rememberInvitationLabel(
  profile: string,
  account: string,
  operationId: string,
  entry: InvitationLabel,
): void {
  const all = readAll();
  all[key(profile, account, operationId)] = entry;
  writeAll(all);
}

export function invitationLabel(
  profile: string,
  account: string,
  operationId: string,
): InvitationLabel | undefined {
  return readAll()[key(profile, account, operationId)];
}

export function forgetInvitationLabel(
  profile: string,
  account: string,
  operationId: string,
): void {
  const all = readAll();
  delete all[key(profile, account, operationId)];
  writeAll(all);
}

export function relativeTime(then: number, now = Date.now()): string {
  const seconds = Math.max(0, Math.round((now - then) / 1000));
  if (seconds < 60) return 'just now';
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return hours === 1 ? '1 hour ago' : `${hours} hours ago`;
  const days = Math.round(hours / 24);
  if (days === 1) return 'yesterday';
  if (days < 14) return `${days} days ago`;
  const weeks = Math.round(days / 7);
  return weeks === 1 ? '1 week ago' : `${weeks} weeks ago`;
}
