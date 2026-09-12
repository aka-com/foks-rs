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
