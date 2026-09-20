import {
  createContext,
  useContext,
  useEffect,
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
    else service.pause();
  }, [service, enabled]);
  // Agent readiness can come and go while this window still owns its queue.
  // Only disposing the owner should erase those submissions.
  useLayoutEffect(() => () => service.stop(), [service]);
  // The send queue is this session's memory alone. The native window asks
  // before discarding it, so it is told how many submissions are only here,
  // and told none once the unlocked shell that owns them goes.
  useEffect(() => {
    if (!bridge.native) return;
    let reported = -1;
    // Reports are chained rather than issued in parallel: a later count must
    // never be overtaken by an earlier one, least of all by the closing zero.
    let last: Promise<unknown> = Promise.resolve();
    const report = (count: number) => {
      if (count === reported) return;
      reported = count;
      last = last
        .then(() => bridge.setUnsentMessages(count))
        .catch(() => {
          // A lost report must not leave the window holding a stale count:
          // the next publication reports this one again.
          if (reported === count) reported = -1;
        });
    };
    report(service.unsentCount());
    const stop = service.subscribe(() => report(service.unsentCount()));
    return () => {
      stop();
      report(0);
    };
  }, [bridge, service]);
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
