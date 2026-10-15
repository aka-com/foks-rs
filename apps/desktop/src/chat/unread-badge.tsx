import { useEffect, useSyncExternalStore } from 'react';
import type { Bridge } from '../bridge';
import type { AgentSnapshot, StoreRef } from '../model';
import type { TeamInbox } from './inbox-service';
import { useLocalAlerts } from './notification-provider';
import { chatUnreadTotal } from './unread';

const empty: ReadonlyMap<StoreRef, TeamInbox> = new Map();
const noSubscribe = () => () => {};
const emptySnapshot = () => empty;

/**
 * The count the app icon shows: the number in the rail's Chat total, which
 * leaves out muted and hidden channels and still counts what is known while
 * a team's synchronization is incomplete; 0 when no number is known.
 */
export function appIconCount(
  snapshot: AgentSnapshot,
  inbox: ReadonlyMap<StoreRef, TeamInbox>,
): number {
  const total = chatUnreadTotal(snapshot, inbox);
  if (!total || !('label' in total)) return 0;
  const count = Number.parseInt(total.label, 10);
  return Number.isNaN(count) ? 0 : count;
}

/**
 * Keeps the app icon's unread count equal to the rail's Chat total, and
 * removes it when the unlocked shell unmounts. While the "Show unread count
 * on app icon" setting is off the count sent is 0; the native side also
 * removes the count when the setting is turned off and when the app locks.
 */
export function UnreadBadge({
  bridge,
  snapshot,
}: {
  bridge: Bridge;
  snapshot: AgentSnapshot;
}) {
  const alerts = useLocalAlerts();
  const service = alerts?.service;
  const inbox = useSyncExternalStore(
    service?.subscribe ?? noSubscribe,
    service?.getSnapshot ?? emptySnapshot,
    service?.getSnapshot ?? emptySnapshot,
  );
  const session = alerts?.session ?? null;
  const count =
    session && session.settings.badge !== false
      ? appIconCount(snapshot, inbox)
      : 0;
  // Sent again for each new session, since the setting may have changed with
  // it. While a setting saves there is no session, and the count last sent
  // stays.
  useEffect(() => {
    if (!session) return;
    void bridge.setUnreadBadge(count).catch(() => {});
  }, [bridge, session, count]);
  useEffect(
    () => () => {
      void bridge.setUnreadBadge(0).catch(() => {});
    },
    [bridge],
  );
  return null;
}
