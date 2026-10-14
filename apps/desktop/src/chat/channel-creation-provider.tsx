import {
  createContext,
  useContext,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react';
import type { ReactNode } from 'react';
import type { Bridge } from '../bridge';
import type { AgentSnapshot, StoreRef } from '../model';
import { Button } from '../components';
import { failure } from './actions';
import { channelTitle } from './presentation';
import {
  ChannelCreationController,
  type ChannelCreationAccess,
} from './channel-creation';
import { useChatInbox, useSidebarInbox } from './inbox-provider';

const Context = createContext<{
  controller: ChannelCreationController;
  access: ChannelCreationAccess;
} | null>(null);
const empty: ReturnType<ChannelCreationController['getSnapshot']> = [];
const emptySnapshot = () => empty;
const noChecks: ReturnType<ChannelCreationController['getChecks']> = new Map();
const emptyChecks = () => noChecks;
const noSubscribe = () => () => {};

export function ChannelCreationProvider({
  bridge,
  snapshot,
  accessNow,
  accessGenerations,
  children,
}: {
  bridge: Bridge;
  snapshot: AgentSnapshot;
  accessNow?: () => number;
  accessGenerations?: ReadonlyMap<string, number>;
  children: ReactNode;
}) {
  const { service } = useChatInbox();
  const access = useMemo(
    () => ({ snapshot, accessNow, accessGenerations }),
    [snapshot, accessNow, accessGenerations],
  );
  // Requests read the access of the last commit when they run. A render
  // reads the access it renders with, from the context value.
  const current = useRef(access);
  useLayoutEffect(() => {
    current.current = access;
  }, [access]);
  const controller = useMemo(
    () => new ChannelCreationController(bridge, service, current),
    [bridge, service],
  );
  useEffect(() => () => controller.dispose(), [controller]);
  const value = useMemo(() => ({ controller, access }), [controller, access]);
  return <Context.Provider value={value}>{children}</Context.Provider>;
}

export function useChannelCreation() {
  const context = useContext(Context);
  const controller = context?.controller ?? null;
  const access = context?.access;
  const creations = useSyncExternalStore(
    controller?.subscribe ?? noSubscribe,
    controller?.getSnapshot ?? emptySnapshot,
    controller?.getSnapshot ?? emptySnapshot,
  );
  const checks = useSyncExternalStore(
    controller?.subscribe ?? noSubscribe,
    controller?.getChecks ?? emptyChecks,
    controller?.getChecks ?? emptyChecks,
  );
  // Whether the team's saved creations were read under the access this
  // render holds.
  const readyFor = useCallback(
    (store: string) =>
      Boolean(controller && access && controller.readyFor(store, access)),
    [controller, access],
  );
  return { controller, creations, checks, readyFor };
}

export function ChannelCreationCompletions({
  onOpen,
}: {
  onOpen: (ref: StoreRef, channel: string) => void;
}) {
  const { controller, creations } = useChannelCreation();
  const [error, setError] = useState('');
  const inbox = useSidebarInbox();
  const completed = creations.filter((record) => record.state === 'confirmed');
  if (!controller || !completed.length) return null;
  return (
    <section
      className="chat-creation-completions"
      aria-label="Created channels"
    >
      {completed.map((record) => {
        const channel = inbox
          .get(record.store.id)
          ?.data?.channels.find(
            (candidate) => candidate.id === record.operation?.channel,
          );
        return (
          <div className="chat-creation-completion" key={record.id}>
            <span>
              {channel
                ? channelTitle(channel)
                : `Channel ${record.operation?.channel.slice(0, 8)}`}{' '}
              was created in {record.store.name}.
            </span>
            <Button
              onClick={() => {
                setError('');
                try {
                  const destination = controller.completedDestination(
                    record.id,
                  );
                  if (!destination) return;
                  onOpen(destination.ref, destination.channel);
                  controller.acknowledge(record.id);
                } catch (cause) {
                  setError(failure(cause));
                }
              }}
            >
              Open channel
            </Button>
            <Button onClick={() => controller.acknowledge(record.id)}>
              Dismiss
            </Button>
          </div>
        );
      })}
      {error && (
        <p className="action-error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
