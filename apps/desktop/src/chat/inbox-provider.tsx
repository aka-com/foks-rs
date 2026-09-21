import type { Location } from '../location';
import { NotificationProvider } from './notification-provider';
import { ChatSendProvider } from './send-provider';
import { ChannelCreationProvider } from './channel-creation-provider';
import {
  createContext,
  useContext,
  useEffect,
  useLayoutEffect,
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
  accessGenerations,
  enabled = true,
  onCatalogRequired,
}: {
  enabled?: boolean;
  bridge: Bridge;
  snapshot: AgentSnapshot;
  children: ReactNode;
  onNavigate?: (location: Location) => void;
  /**
   * Requests a profile's catalog refresh when a team's read says the
   * profile's vault has not been refreshed. Without it the read's error is
   * shown and the service waits for the next refresh on its own.
   */
  onCatalogRequired?: (profile: string) => void;
  clock?: ChatClock;
  /**
   * The shell's availability clock in seconds. The service decides which teams
   * it keeps on the same clock the Chat tab decides reachability on, so the two
   * cannot disagree about a check-in that expired this second.
   */
  accessNow?: () => number;
  accessGenerations?: ReadonlyMap<string, number>;
}) {
  const service = useMemo(
    () => new ChatInboxService(bridge, clock),
    [bridge, clock],
  );
  useLayoutEffect(() => {
    service.onCatalogRequired = onCatalogRequired;
  }, [service, onCatalogRequired]);
  useLayoutEffect(() => {
    if (enabled)
      service.updateStores(
        snapshot,
        accessNow ? { nowSeconds: accessNow() } : {},
        accessGenerations,
      );
    else service.histories.clear();
  }, [service, snapshot, accessNow, accessGenerations, enabled]);
  useEffect(() => {
    if (enabled) service.start();
    return () => service.stop();
  }, [service, enabled]);
  return (
    <Context.Provider value={service}>
      <NotificationProvider
        bridge={bridge}
        service={service}
        onNavigate={onNavigate}
      >
        <ChatSendProvider
          enabled={enabled}
          bridge={bridge}
          snapshot={snapshot}
          inbox={service}
          clock={clock}
          accessGenerations={accessGenerations}
        >
          <ChannelCreationProvider
            bridge={bridge}
            snapshot={snapshot}
            accessNow={accessNow}
            accessGenerations={accessGenerations}
          >
            {children}
          </ChannelCreationProvider>
        </ChatSendProvider>
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
