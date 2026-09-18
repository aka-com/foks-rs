import {
  createContext,
  useContext,
  useEffect,
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
import { ChannelCreationController } from './channel-creation';
import { useChatInbox, useSidebarInbox } from './inbox-provider';

const Context = createContext<ChannelCreationController | null>(null);
const empty: ReturnType<ChannelCreationController['getSnapshot']> = [];
const emptySnapshot = () => empty;
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
  const current = useRef({ snapshot, accessNow, accessGenerations });
  current.current = { snapshot, accessNow, accessGenerations };
  const controller = useMemo(
    () => new ChannelCreationController(bridge, service, () => current.current),
    [bridge, service],
  );
  useEffect(() => () => controller.dispose(), [controller]);
  return <Context.Provider value={controller}>{children}</Context.Provider>;
}

export function useChannelCreation() {
  const controller = useContext(Context);
  const creations = useSyncExternalStore(
    controller?.subscribe ?? noSubscribe,
    controller?.getSnapshot ?? emptySnapshot,
    controller?.getSnapshot ?? emptySnapshot,
  );
  return { controller, creations };
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
