import type { TrackedOperation } from './operations';

/** Shared by automatic and explicit status checks within one owner lifetime. */
export function coalesceStatus<T>(
  checks: Map<string, Promise<T>>,
  id: string,
  send: () => Promise<T>,
): Promise<T> {
  const existing = checks.get(id);
  if (existing) return existing;
  const work = send().finally(() => {
    if (checks.get(id) === work) checks.delete(id);
  });
  checks.set(id, work);
  return work;
}

type Lane = 'status' | 'body';
type Entry = { served: number; next: number; failures: number };
const terminal = (op: TrackedOperation) =>
  ['confirmed', 'cancelled', 'rejected'].includes(op.state);

/** Volatile metadata only; a new owner receives a new schedule. */
export class RecoverySchedule {
  private entries = {
    status: new Map<string, Entry>(),
    body: new Map<string, Entry>(),
  };
  private turn = 0;
  constructor(private readonly now: () => number = () => performance.now()) {}

  select(
    lane: Lane,
    rows: readonly TrackedOperation[],
    visited: ReadonlySet<string>,
    blocked: (channel: string) => boolean,
  ): TrackedOperation | undefined {
    const eligible = rows.filter(
      (op) =>
        !op.observed &&
        !blocked(op.channel) &&
        (lane === 'status'
          ? !terminal(op) && (op.statusUnknown || op.state === 'uncertain')
          : op.kind === 'send-message' &&
            !op.bodyUnavailable &&
            op.text === undefined),
    );
    const ids = new Set(eligible.map((op) => op.id));
    const entries = this.entries[lane];
    for (const id of entries.keys()) if (!ids.has(id)) entries.delete(id);
    // New arrivals join the tail, rather than forever outranking existing work.
    for (const id of ids)
      if (!entries.has(id))
        entries.set(id, { served: ++this.turn, next: 0, failures: 0 });
    const now = this.now();
    return eligible
      .filter(
        (op) => !visited.has(op.id) && (entries.get(op.id)?.next ?? 0) <= now,
      )
      .sort(
        (a, b) =>
          (entries.get(a.id)?.served ?? 0) - (entries.get(b.id)?.served ?? 0),
      )[0];
  }

  complete(lane: Lane, id: string, failed: boolean) {
    const failures = failed
      ? Math.min((this.entries[lane].get(id)?.failures ?? 0) + 1, 6)
      : 0;
    this.entries[lane].set(id, {
      served: ++this.turn,
      failures,
      next:
        this.now() +
        (failed ? Math.min(1000 * 2 ** (failures - 1), 30000) : 2000),
    });
  }
}
