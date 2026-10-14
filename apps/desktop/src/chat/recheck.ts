/** A channel the reader rechecked from its stopped pane. */
export interface Recheck {
  /** When the channel was blocked again, if it was. */
  stopped: number | null;
  /** The team's `authorizationRevision` when the reader rechecked. */
  revision: number;
  /** Whether a history page of the channel was accepted since. */
  read: boolean;
}

/**
 * Advances the open rechecks against the team's blocked set and its
 * `authorizationRevision`. A channel blocked again records when, which the
 * stopped pane states as the recheck's outcome. A channel that is still
 * unblocked after a synchronization accepted since the recheck, and whose
 * history has been accepted since the recheck, has verified: its entry is
 * dropped, so a later block is not reported as that recheck's outcome. `read`
 * names the channel whose history page was just accepted.
 */
export function settleRechecks(
  old: ReadonlyMap<string, Recheck>,
  blocked: ReadonlySet<string>,
  revision: number,
  read?: string,
): ReadonlyMap<string, Recheck> {
  let next: Map<string, Recheck> | null = null;
  for (const [channel, entry] of old) {
    if (entry.stopped !== null) continue;
    const wasRead = entry.read || channel === read;
    const value = blocked.has(channel)
      ? { ...entry, stopped: Date.now() }
      : wasRead && revision > entry.revision
        ? null
        : wasRead !== entry.read
          ? { ...entry, read: true }
          : entry;
    if (value === entry) continue;
    next ??= new Map(old);
    if (value) next.set(channel, value);
    else next.delete(channel);
  }
  return next ?? old;
}
