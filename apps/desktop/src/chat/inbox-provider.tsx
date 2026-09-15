import type { Location } from '../location';
import { NotificationProvider } from './notification-provider';
import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useSyncExternalStore,
} from 'react';
import type { ReactNode } from 'react';
import type { Bridge } from '../bridge';
import type { AgentSnapshot } from '../model';
import { ChatInboxService } from './inbox-service';
import type { ChatClock } from './inbox-service';
const Context = createContext<ChatInboxService | null>(null);
export function ChatInboxProvider({
  bridge,
  snapshot,
  children,
  onNavigate,
  clock,
  accessNow,
}: {
  bridge: Bridge;
  snapshot: AgentSnapshot;
  children: ReactNode;
  onNavigate?: (location: Location) => void;
  clock?: ChatClock;
  /**
   * The shell's availability clock in seconds. The service decides which teams
   * it keeps on the same clock the Chat tab decides reachability on, so the two
   * cannot disagree about a check-in that expired this second.
   */
  accessNow?: () => number;
}) {
  const service = useMemo(
    () => new ChatInboxService(bridge, clock),
    [bridge, clock],
  );
  useEffect(
    () =>
      service.updateStores(
        snapshot,
        accessNow ? { nowSeconds: accessNow() } : {},
      ),
    [service, snapshot, accessNow],
  );
  useEffect(() => {
    service.start();
    return () => service.stop();
  }, [service]);
  return (
    <Context.Provider value={service}>
      <NotificationProvider
        bridge={bridge}
        service={service}
        onNavigate={onNavigate}
      >
        {children}
      </NotificationProvider>
    </Context.Provider>
  );
}
export function useChatInbox() {
  const service = useContext(Context);
  if (!service)
    throw new Error('Chat requires the unlocked shell inbox provider.');
  const snapshot = useSyncExternalStore(
    service.subscribe,
    service.getSnapshot,
    service.getSnapshot,
  );
  return { service, snapshot };
}
const empty: ReturnType<ChatInboxService['getSnapshot']> = new Map();
const noSubscribe = () => () => {};
const emptySnapshot = () => empty;
/** Sidebar also renders in static shell fixtures outside an unlocked provider. */
export function useSidebarInbox() {
  const service = useContext(Context);
  return useSyncExternalStore(
    service?.subscribe ?? noSubscribe,
    service?.getSnapshot ?? emptySnapshot,
    service?.getSnapshot ?? emptySnapshot,
  );
}
