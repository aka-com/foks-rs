/**
 * When each recovery phrase was last tested on this device.
 *
 * A record holds times only, never phrase material: when a typed copy last
 * matched a backup key, and until when the overdue reminder is snoozed. It is
 * keyed by the backup key's entity id, which names one key on one server
 * whatever local alias the account carries, and kept in the webview's local
 * storage beside the app's other nonsecret per-device records, so it survives
 * restarts but is not shared with other devices. Only a full test on Devices
 * writes `testedAt`; the three-token check when a phrase is created does not.
 */

import { STORAGE_KEYS, storedRecord } from './platform/device-storage';

const DAY_MS = 24 * 60 * 60 * 1000;

/** A phrase last tested longer ago than this is due for another test. */
export const RECOVERY_TEST_INTERVAL_DAYS = 180;
/** How long "Remind me later" hides a phrase's reminder. */
export const RECOVERY_SNOOZE_DAYS = 30;

export interface RecoveryPhraseTestRecord {
  testedAt?: number;
  snoozedUntil?: number;
}

type Stored = Readonly<Record<string, RecoveryPhraseTestRecord>>;

function time(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) && value > 0
    ? value
    : undefined;
}

function parse(raw: string | null): Stored {
  if (!raw) return {};
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed))
      return {};
    const result: Record<string, RecoveryPhraseTestRecord> = {};
    for (const [name, value] of Object.entries(parsed)) {
      if (!value || typeof value !== 'object') continue;
      const record = value as Record<string, unknown>;
      const testedAt = time(record.testedAt);
      const snoozedUntil = time(record.snoozedUntil);
      result[name] = {
        ...(testedAt ? { testedAt } : {}),
        ...(snoozedUntil ? { snoozedUntil } : {}),
      };
    }
    return result;
  } catch {
    return {};
  }
}

const records = storedRecord(STORAGE_KEYS.recoveryPhraseTests, parse, (value) =>
  JSON.stringify(value),
);

/**
 * The current records. The value is reused while the stored text is
 * unchanged, so React can compare snapshots by identity.
 */
export function recoveryPhraseTests(): Stored {
  return records.read();
}

export function subscribeRecoveryPhraseTests(listener: () => void): () => void {
  return records.subscribe(listener);
}

function update(
  backupId: string,
  change: (record: RecoveryPhraseTestRecord) => RecoveryPhraseTestRecord,
): boolean {
  const all = { ...recoveryPhraseTests() };
  all[backupId] = change(all[backupId] ?? {});
  // Whether the records were stored; blocked or full storage refuses them.
  return records.write(all);
}

/**
 * Records that a typed copy matched this backup key; clears any snooze.
 * Returns whether the record was stored, so a caller does not report a test
 * that the next read will not find.
 */
export function recordRecoveryPhraseTest(
  backupId: string,
  at = Date.now(),
): boolean {
  return update(backupId, () => ({ testedAt: at }));
}

/** Hides this phrase's reminder for {@link RECOVERY_SNOOZE_DAYS}. */
export function snoozeRecoveryPhraseReminder(
  backupId: string,
  at = Date.now(),
): void {
  update(backupId, (record) => ({
    ...record,
    snoozedUntil: at + RECOVERY_SNOOZE_DAYS * DAY_MS,
  }));
}

/**
 * - `never`: no copy has matched on this device.
 * - `current`: the last match is within the test interval.
 * - `overdue`: the last match is older than the interval.
 *
 * `snoozed` says whether the reminder for a phrase that is due is hidden for
 * now.
 */
export type RecoveryPhraseTestStatus =
  | { state: 'never'; snoozed: boolean }
  | { state: 'current'; testedAt: number }
  | { state: 'overdue'; testedAt: number; snoozed: boolean };

export function recoveryPhraseTestStatus(
  records: Stored,
  backupId: string,
  now = Date.now(),
): RecoveryPhraseTestStatus {
  const record = records[backupId];
  const testedAt = record?.testedAt;
  const snoozed = (record?.snoozedUntil ?? 0) > now;
  if (testedAt === undefined) return { state: 'never', snoozed };
  if (now - testedAt <= RECOVERY_TEST_INTERVAL_DAYS * DAY_MS)
    return { state: 'current', testedAt };
  return { state: 'overdue', testedAt, snoozed };
}

/** Whether the status calls for the Devices dot and band: never tested or overdue, and not snoozed. */
export function recoveryPhraseTestDue(
  status: RecoveryPhraseTestStatus,
): status is Exclude<RecoveryPhraseTestStatus, { state: 'current' }> {
  return status.state !== 'current' && !status.snoozed;
}

/** The device's local date, as "Mar 3, 2026". */
export function testedDate(at: number): string {
  return new Date(at).toLocaleDateString(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  });
}
