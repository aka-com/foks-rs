/**
 * Whether the agent admits new MCP data writes for one account. `paused`
 * means the agent's anchor disagrees with this computer's clock, and writes
 * resume only after a person confirms the current time. `behind` means the
 * clock is trusted but behind a watermark committed while an earlier clock
 * ran ahead; writes resume at `resumesAt`, and confirming the time does not
 * bring that forward.
 */
export const adapterClockAdmissions = [
  'unanchored',
  'trusted',
  'paused',
  'behind',
] as const;
export interface AdapterClock {
  admission: (typeof adapterClockAdmissions)[number];
  /** The computer's wall time when the agent checked, in Unix seconds. */
  localTime: number;
  /** When admission is `behind`, the Unix time at which writes resume. */
  resumesAt: number | null;
}
function unixSeconds(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
}
export function decodeAdapterClock(value: unknown): AdapterClock {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    throw new Error('Invalid adapter clock status');
  const v = value as Record<string, unknown>;
  if (
    Object.keys(v).sort().join(',') !== 'admission,localTime,resumesAt' ||
    typeof v.admission !== 'string' ||
    !(adapterClockAdmissions as readonly string[]).includes(v.admission) ||
    !unixSeconds(v.localTime) ||
    (v.admission === 'behind'
      ? !unixSeconds(v.resumesAt)
      : v.resumesAt !== null)
  )
    throw new Error('Invalid adapter clock status');
  return value as AdapterClock;
}
