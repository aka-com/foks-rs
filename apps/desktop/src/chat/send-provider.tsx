import {
  createContext,
  useContext,
  useLayoutEffect,
  useMemo,
  useRef,
  useSyncExternalStore,
} from 'react';
import type { ReactNode } from 'react';
import type { Bridge } from '../bridge';
import type { AgentSnapshot } from '../model';
import type { ChatClock, ChatInboxService } from './inbox-service';
import { ChatSendService } from './send-service';
import { diagnosticLog, hashId } from '../diagnostics/log';

const Context = createContext<ChatSendService | null>(null);
export function ChatSendProvider({
  bridge,
  snapshot,
  inbox,
  clock,
  accessGenerations,
  children,
  enabled = true,
}: {
  enabled?: boolean;
  bridge: Bridge;
  snapshot: AgentSnapshot;
  inbox: ChatInboxService;
  clock?: ChatClock;
  accessGenerations?: ReadonlyMap<string, number>;
  children: ReactNode;
}) {
  const initial = useRef(snapshot);
  initial.current = snapshot;
  const service = useMemo(() => {
    const owner = new ChatSendService(bridge, inbox, clock);
    owner.update(initial.current);
    return owner;
  }, [bridge, inbox, clock]);
  useLayoutEffect(() => {
    service.update(snapshot, accessGenerations);
  }, [service, snapshot, accessGenerations]);
  useLayoutEffect(() => {
    if (enabled) service.start();
    return () => service.stop();
  }, [service, enabled]);
  useLayoutEffect(
    () =>
      service.observe((event) =>
        diagnosticLog.record({
          name: 'chat.send',
          scope: `store#${hashId(event.store)}`,
          id: hashId(event.message),
          phase: event.step,
          ms: event.milliseconds,
          outcome: event.step === 'failed' ? 'error' : 'ok',
          code: event.code,
        }),
      ),
    [service],
  );
  return <Context.Provider value={service}>{children}</Context.Provider>;
}
export function useChatSends() {
  const service = useContext(Context);
  if (!service) throw new Error('Chat requires the unlocked send provider.');
  const revision = useSyncExternalStore(
    service.subscribe,
    service.getSnapshot,
    service.getSnapshot,
  );
  return { service, revision };
}
