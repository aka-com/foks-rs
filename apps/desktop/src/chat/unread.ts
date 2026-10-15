import { chatAvailable, storeNavigationOrder } from '../model';
import type { AgentSnapshot, StoreRef } from '../model';
import type { TeamInbox } from './inbox-service';
export function teamUnread(
  entry: TeamInbox | undefined,
): { label: string; description: string } | null {
  if (!entry || (entry.state === 'loading' && !entry.data && !entry.error))
    return { label: '…', description: 'Loading unread count' };
  if (entry.state === 'blocked' || entry.state === 'unavailable')
    return { label: '!', description: 'Chat unavailable' };
  if (!entry.data)
    return { label: '!', description: 'Unread count unavailable' };
  const count = entry.data.conversations.reduce(
    (sum, c) => sum + (c.hidden || c.muted ? 0n : BigInt(c.unread)),
    0n,
  );
  const label = count > 999n ? '999+' : String(count);
  const incomplete = entry.data.degraded;
  if (incomplete)
    return {
      label: count ? `${label}${count > 999n ? '' : '+'}` : '?',
      description: `${count} known unread; inbox synchronization incomplete`,
    };
  if (entry.stale)
    return {
      label: count ? `${label}·` : '!',
      description: `${count} unread; count may be out of date`,
    };
  return count
    ? {
        label,
        description: `${count} unread`,
      }
    : null;
}

/**
 * The total unread across the teams whose chat this device can read, which
 * the rail's Chat tab and the app icon show. `teamUnread`
 * reports one team at a time, including the states that have no number
 * (loading, unavailable); those contribute nothing to the sum. Loading is
 * reported by the header Refresh control.
 *
 * A team whose synchronization failed or arrived incomplete marks the badge
 * rather than replacing it: an inbox poll fails and retries on a backoff, and
 * a count that vanished on each failed poll would read as messages going away.
 * The count that is known is still drawn, in the warning colour, and the
 * description says what is unaccounted for. Only when no team could be counted
 * at all does the badge fall back to a warning dot with nothing to show.
 */
export function chatUnreadTotal(
  snapshot: AgentSnapshot,
  inbox: ReadonlyMap<StoreRef, TeamInbox>,
):
  | { label: string; description: string; warning?: boolean }
  | { warning: true; description: string }
  | null {
  let total = 0;
  let known = false;
  const warnings = new Set<string>();
  for (const store of storeNavigationOrder(snapshot)) {
    if (!chatAvailable(snapshot, store)) continue;
    const entry = inbox.get(store.id);
    if (!entry || (entry.state === 'loading' && !entry.data && !entry.error)) {
      continue;
    }
    const unread = teamUnread(entry);
    if (!unread) continue;
    if (
      !entry.data ||
      entry.state === 'blocked' ||
      entry.state === 'unavailable' ||
      entry.stale ||
      entry.error ||
      entry.data.degraded
    )
      warnings.add(unread.description);
    const count = Number.parseInt(unread.label, 10);
    if (!Number.isNaN(count)) {
      total += count;
      known = true;
    }
  }
  if (warnings.size) {
    const description = `${known ? `${total} known unread; ` : ''}${[...warnings].join('; ')}`;
    return known
      ? { label: String(total), description, warning: true }
      : { warning: true, description };
  }
  if (known) return { label: String(total), description: `${total} unread` };
  return null;
}
