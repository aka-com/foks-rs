import { CardSelect, Inset, InsetRow } from '../components';
import { chatClient, sameScope } from './client';
import { failure } from './actions';
import { normalizeCommandError } from '../bridge';
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
import { diagnosticLog } from '../diagnostics/log';
import { notificationKey } from './local-contract';
/** The agent's code for alerts the system has not permitted. */
const PERMISSION_CODE = 'chat-notification-permission';

const Context = createContext<{
  session: LocalSession | null;
  error: string;
  /** True when the error is the system refusing alerts, which has a way out. */
  permission: boolean;
  /** Opens the system pane where that refusal is undone. */
  openSettings: () => Promise<void>;
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
    [error, setError] = useState(''),
    [permission, setPermission] = useState(false);
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
      undefined,
      (event) => {
        // A pass is timed and counted; the messages it read are not named.
        if (event.kind === 'pass')
          diagnosticLog.record({
            name: 'chat.notify',
            ms: event.milliseconds,
            outcome: 'ok',
            attrs: {
              rows: event.rows,
              bytes: event.bytes,
              budgetHit: event.budgetHit === true,
              candidates: event.candidates.length,
              baseline: event.baselineOnly,
              incomplete: event.incomplete,
            },
          });
        else if (event.kind === 'failure')
          diagnosticLog.record({
            name: 'chat.notify',
            outcome: event.cancelled
              ? 'cancelled'
              : event.busy
                ? 'busy'
                : 'error',
            attrs: { retry: event.retry },
          });
      },
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
            kind: 'chat',
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
      setPermission(false);
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
          setPermission(normalizeCommandError(cause).code === PERMISSION_CODE);
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
  const openSettings = useCallback(async () => {
    try {
      await bridge.openNotificationSettings();
    } catch (cause) {
      setError(failure(cause));
      setPermission(false);
    }
  }, [bridge]);
  return (
    <Context.Provider
      value={{ session, error, permission, openSettings, configure }}
    >
      {children}
    </Context.Provider>
  );
}
export function NotificationSettings({
  storeId,
  scope,
  channel,
}: {
  storeId?: string;
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
  const { session, error, permission, openSettings, configure } = context;
  const override = key ? session?.settings.overrides[key] : undefined;
  const mode = override === undefined ? 'inherit' : override ? 'all' : 'none';
  return (
    <div
      className={`chat-local-settings${storeId ? '' : ' device-notification-settings'}`}
    >
      {!session ? <p role="status">Checking local alert settings…</p> : null}
      {!storeId && (
        <Inset className="notification-preferences settings-checkboxes">
          <label>
            <input
              type="checkbox"
              disabled={!session?.available}
              checked={session?.settings.enabled ?? false}
              onChange={(e) =>
                void configure({
                  action: 'configure',
                  enabled: e.target.checked,
                })
              }
            />
            <span>Enable desktop alerts on this device</span>
          </label>
          <label>
            <input
              type="checkbox"
              disabled={!session?.available || !session.settings.enabled}
              checked={session?.settings.previews ?? false}
              onChange={(e) =>
                void configure({
                  action: 'configure',
                  previews: e.target.checked,
                })
              }
            />
            <span>Include message previews</span>
          </label>
        </Inset>
      )}
      {storeId && scope && channel && key && (
        <Inset className="chat-alert-mode">
          <InsetRow label="Alerts">
            <CardSelect
              label="Alerts"
              value={mode}
              disabled={!session?.available}
              options={[
                { id: 'inherit', title: 'Use device setting' },
                { id: 'all', title: 'All new messages' },
                { id: 'none', title: 'None' },
              ]}
              onChange={(next) =>
                void configure({
                  action: 'configure',
                  storeId,
                  scope,
                  channel,
                  mode: next === 'inherit' ? null : next === 'all',
                })
              }
            />
          </InsetRow>
        </Inset>
      )}
      {session && !session.available ? (
        <p role="alert">Desktop alerts are unavailable in this build.</p>
      ) : null}
      {error &&
        // The agent states the refusal; the shell says it again with the
        // settings pane attached, because it is the one error with a way out.
        (permission ? (
          <p role="alert">
            Desktop alerts were not permitted. Check{' '}
            <button
              type="button"
              className="chat-alert-link"
              onClick={() => void openSettings()}
            >
              macOS notification settings
            </button>
            .
          </p>
        ) : (
          <p role="alert">{error}</p>
        ))}
    </div>
  );
}
