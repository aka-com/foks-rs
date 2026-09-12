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
import type { World } from '../model';
import { ChatInboxService } from './inbox-service';
const Context = createContext<ChatInboxService | null>(null);
export function ChatInboxProvider({
  bridge,
  world,
  children,
  onNavigate,
}: {
  bridge: Bridge;
  world: World;
  children: ReactNode;
  onNavigate?: (location: Location) => void;
}) {
  const service = useMemo(() => new ChatInboxService(bridge), [bridge]);
  useEffect(() => service.updateStores(world), [service, world]);
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
