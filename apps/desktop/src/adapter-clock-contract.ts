/**
 * Whether the agent admits new MCP data writes for one account. `paused`
 * means the agent's anchor disagrees with this computer's clock, and writes
 * resume only after a person confirms the current time.
 */
export const adapterClockAdmissions = [
  'unanchored',
  'trusted',
  'paused',
] as const;
export interface AdapterClock {
  admission: (typeof adapterClockAdmissions)[number];
  /** The computer's wall time when the agent checked, in Unix seconds. */
  localTime: number;
}
export function decodeAdapterClock(value: unknown): AdapterClock {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    throw new Error('Invalid adapter clock status');
  const v = value as Record<string, unknown>;
  if (
    Object.keys(v).sort().join(',') !== 'admission,localTime' ||
    typeof v.admission !== 'string' ||
    !(adapterClockAdmissions as readonly string[]).includes(v.admission) ||
    typeof v.localTime !== 'number' ||
    !Number.isSafeInteger(v.localTime) ||
    v.localTime < 0
  )
    throw new Error('Invalid adapter clock status');
  return value as AdapterClock;
}
