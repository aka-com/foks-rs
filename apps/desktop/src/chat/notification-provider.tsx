import { chatClient, sameScope } from './client';
import { failure } from './actions';
import type { Location } from '../location';
import {
  createContext,
  useContext,
  useEffect,
  useState,
  useCallback,
  useRef,
} from 'react';
import type { ReactNode } from 'react';
import type { Bridge } from '../bridge';
import type { ChatScope } from '../chat-contract';
import type { ChatInboxService } from './inbox-service';
import type { LocalAction, LocalSession } from './local-contract';
import { NotificationConsumer } from './notification-consumer';
import { notificationKey } from './local-contract';
const Context = createContext<{
  session: LocalSession | null;
  error: string;
  configure: (
    action: Extract<LocalAction, { action: 'configure' }>,
  ) => Promise<void>;
} | null>(null);
export function NotificationProvider({
  bridge,
  service,
  children,
  onNavigate,
}: {
  bridge: Bridge;
  service: ChatInboxService;
  children: ReactNode;
  onNavigate?: (location: Location) => void;
}) {
  const [session, setSession] = useState<LocalSession | null>(null),
    [error, setError] = useState('');
  const lifetime = useRef({ active: false });
  const configuring = useRef(false);
  useEffect(() => {
    const owner = { active: true };
    lifetime.current = owner;
    let active = true;
    let epoch: string | undefined;
    void bridge
      .chatLocal({ action: 'begin' })
      .then((reply) => {
        epoch = reply.epoch;
        if (active) setSession(reply);
        else void bridge.chatLocal({ action: 'end', epoch }).catch(() => {});
      })
      .catch(() => {
        if (active) setError('Local alerts are unavailable.');
      });
    return () => {
      active = false;
      owner.active = false;
      if (epoch)
        void bridge.chatLocal({ action: 'end', epoch }).catch(() => {});
    };
  }, [bridge]);
  useEffect(() => {
    if (!session) return;
    const consumer = new NotificationConsumer(
      bridge,
      service,
      session,
      setError,
    );
    return () => {
      consumer.stop();
      void bridge
        .chatLocal({ action: 'end', epoch: session.epoch })
        .catch(() => {});
    };
  }, [bridge, service, session]);
  useEffect(() => {
    if (!session || !onNavigate) return;
    let active = true;
    let stop: (() => void) | undefined;
    let client: ReturnType<typeof chatClient> | undefined;
    const activate = async () => {
      try {
        const reply = await bridge.chatLocal({ action: 'take-activation' });
        if (!active || !reply.activation) return;
        const route = reply.activation;
        client?.dispose();
        client = chatClient(bridge, route.scope.store.profile, route.storeId);
        const history = await client.request({
          action: 'history',
          channel: route.channel,
          before: null,
        });
        if (active && sameScope(history.scope, route.scope))
          onNavigate({
            kind: 'team-chat',
            ref: route.storeId,
            channel: route.channel,
          });
      } catch {
        if (active) setError('This notification is no longer available.');
      }
    };
    void bridge
      .onChatNotification((kind) => {
        if (kind === 'error') setError('Desktop alert could not be delivered.');
        else void activate();
      })
      .then((off) => {
        if (active) {
          stop = off;
          void activate();
        } else off();
      })
      .catch(() => {
        if (active) setError('Notification activation is unavailable.');
      });
    return () => {
      active = false;
      stop?.();
      client?.dispose();
    };
  }, [bridge, onNavigate, session]);
  const configure = useCallback(
    async (action: Extract<LocalAction, { action: 'configure' }>) => {
      if (configuring.current) return;
      configuring.current = true;
      const owner = lifetime.current;
      setError('');
      setSession(null);
      try {
        const next = await bridge.chatLocal(action);
        if (owner.active && lifetime.current === owner) setSession(next);
        else
          void bridge
            .chatLocal({ action: 'end', epoch: next.epoch })
            .catch(() => {});
      } catch (cause) {
        if (owner.active && lifetime.current === owner) {
          setError(failure(cause));
          // A permission or persistence failure must leave controls retryable.
          // Begin reloads the native settings actually committed, with a fresh
          // baseline; it never repeats the user's configuration mutation.
          try {
            const restored = await bridge.chatLocal({ action: 'begin' });
            if (owner.active && lifetime.current === owner)
              setSession(restored);
            else
              void bridge
                .chatLocal({ action: 'end', epoch: restored.epoch })
                .catch(() => {});
          } catch {
            if (owner.active && lifetime.current === owner)
              setError(
                failure(cause) + ' Local settings could not be reloaded.',
              );
          }
        }
      } finally {
        configuring.current = false;
      }
    },
    [bridge],
  );
  return (
    <Context.Provider value={{ session, error, configure }}>
      {children}
    </Context.Provider>
  );
}
export function NotificationSettings({
  storeId,
  scope,
  channel,
}: {
  storeId: string;
  scope?: ChatScope;
  channel?: string;
}) {
  const context = useContext(Context);
  const [key, setKey] = useState('');
  useEffect(() => {
    let active = true;
    setKey('');
    if (scope && channel)
      void notificationKey(scope, channel).then((value) => {
        if (active) setKey(value);
      });
    return () => {
      active = false;
    };
  }, [scope, channel]);
  if (!context) return null;
  const { session, error, configure } = context;
  return (
    <details className="chat-local-settings">
      <summary>Alerts on this device</summary>
      {!session ? (
        <p role="status">Checking local alert settings…</p>
      ) : !session.available ? (
        <p>Desktop alerts are unavailable in this runtime.</p>
      ) : null}
      <label>
        <input
          type="checkbox"
          disabled={!session?.available}
          checked={session?.settings.enabled ?? false}
          onChange={(e) =>
            void configure({ action: 'configure', enabled: e.target.checked })
          }
        />
        Enable desktop alerts on this device
      </label>
      <label>
        <input
          type="checkbox"
          disabled={!session?.available || !session.settings.enabled}
          checked={session?.settings.previews ?? false}
          onChange={(e) =>
            void configure({ action: 'configure', previews: e.target.checked })
          }
        />
        Include message previews
      </label>
      {scope && channel && key && (
        <label>
          Channel alerts
          <select
            disabled={!session?.available}
            value={
              session?.settings.overrides[key] === undefined
                ? 'inherit'
                : session.settings.overrides[key]
                  ? 'all'
                  : 'none'
            }
            onChange={(e) =>
              void configure({
                action: 'configure',
                storeId,
                scope,
                channel,
                mode:
                  e.target.value === 'inherit'
                    ? null
                    : e.target.value === 'all',
              })
            }
          >
            <option value="inherit">Use device setting</option>
            <option value="all">All new messages</option>
            <option value="none">None</option>
          </select>
        </label>
      )}
      <p>
        While unlocked and running. Muted and hidden channels stay silent.
        Settings do not sync.
      </p>
      {error && <p role="alert">{error}</p>}
    </details>
  );
}
