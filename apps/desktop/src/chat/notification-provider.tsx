import { CardSelect, Icon, Inset, InsetRow } from '../components';
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
  useSyncExternalStore,
} from 'react';
import type { ReactNode } from 'react';
import type { Bridge } from '../bridge';
import type { ChatScope } from '../chat-contract';
import type { FoksIconName } from '../icons';
import type { ChatClock } from './clock';
import type { ChatInboxService } from './inbox-service';
import type { AlertChoice, LocalAction, LocalSession } from './local-contract';
import { NotificationConsumer } from './notification-consumer';
import { diagnosticLog } from '../diagnostics/log';
import {
  alertChoice,
  inheritedAlerts,
  notificationKey,
  storedAlertMode,
} from './local-contract';
/** The agent's code for alerts the system has not permitted. */
const PERMISSION_CODE = 'chat-notification-permission';

const Context = createContext<{
  session: LocalSession | null;
  error: string;
  /** True when the error is the system refusing alerts, which has a way out. */
  permission: boolean;
  /** Opens the system pane where that refusal is undone. */
  openSettings: () => Promise<void>;
  /**
   * Applies a change and resolves to the error it was refused with, or to
   * `''` once it is saved. A change sent while another is saving is dropped
   * and resolves to an error that says so.
   */
  configure: (
    action: Extract<LocalAction, { action: 'configure' }>,
  ) => Promise<string>;
  /** The inbox whose projection carries each channel's `muted` flag. */
  service: ChatInboxService;
} | null>(null);

/**
 * The local alert settings and the way to change them, for controls outside
 * the settings panels (the Chat column's menus); `null` outside the provider.
 */
export function useLocalAlerts() {
  return useContext(Context);
}
export function NotificationProvider({
  bridge,
  service,
  children,
  onNavigate,
  clock,
  username,
}: {
  bridge: Bridge;
  service: ChatInboxService;
  children: ReactNode;
  onNavigate?: (location: Location) => void;
  /** The inbox service's clock, so the consumer's deadlines share its time. */
  clock?: ChatClock;
  /**
   * The reader's username in a store, which the Mentions only mode matches;
   * without it, that mode raises no alerts.
   */
  username?: (storeId: string, scope: ChatScope) => string | null;
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
  // Local mutes reach the published projection, which every unread total
  // reads. While a change is saving there is no session, and the mutes last
  // known stay applied.
  useEffect(() => {
    if (session) service.setLocalMutes(session.settings.muted);
  }, [service, session]);
  useEffect(() => {
    if (!session) return;
    const consumer = new NotificationConsumer(
      bridge,
      service,
      session,
      setError,
      clock,
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
      undefined,
      username,
    );
    return () => {
      consumer.stop();
      void bridge
        .chatLocal({ action: 'end', epoch: session.epoch })
        .catch(() => {});
    };
  }, [bridge, service, session, clock, username]);
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
    async (
      action: Extract<LocalAction, { action: 'configure' }>,
    ): Promise<string> => {
      if (configuring.current) return 'Another local setting is being saved.';
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
        return '';
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
        return failure(cause);
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
      value={{ session, error, permission, openSettings, configure, service }}
    >
      {children}
    </Context.Provider>
  );
}
/** The titles the alert modes are offered under. */
export const ALERT_TITLES: Readonly<Record<AlertChoice, string>> = {
  all: 'All new messages',
  mentions: 'Mentions only',
  none: 'None',
};
/** What Mentions only matches, said where the mode is offered. */
export const MENTIONS_DETAIL =
  'Text containing @ and your username, matched on this device';
/** The detail of the Default choice: the mode it gives and why. */
function inheritedDetail({
  mode,
  from,
}: ReturnType<typeof inheritedAlerts>): string {
  if (from === 'muted') return 'None while the channel is muted';
  if (from === 'team') return `${ALERT_TITLES[mode]}, the team default`;
  return 'Device setting';
}
const noSubscribe = () => () => {};
/** Whether the published projection marks a channel muted. */
function useProjectedMute(
  service: ChatInboxService | undefined,
  storeId: string | undefined,
  channel: string | undefined,
): boolean {
  const read = () =>
    !!service
      ?.getSnapshot()
      .get(storeId ?? '')
      ?.data?.conversations.find(
        (conversation) => conversation.channel.id === channel,
      )?.muted;
  return useSyncExternalStore(service?.subscribe ?? noSubscribe, read, read);
}
export function NotificationSettings({
  storeId,
  scope,
  channel,
  noteIcon,
}: {
  storeId?: string;
  scope?: ChatScope;
  channel?: string;
  /** Optional icon for the unavailable-notifications message. */
  noteIcon?: FoksIconName;
}) {
  const context = useContext(Context);
  const muted = useProjectedMute(context?.service, storeId, channel);
  if (!context) return null;
  const { session, error, permission, openSettings, configure } = context;
  const key = scope && channel ? notificationKey(scope, channel) : '';
  const override = key ? session?.settings.overrides[key] : undefined;
  const mode = override === undefined ? 'inherit' : alertChoice(override);
  const inherited =
    session && scope ? inheritedAlerts(session.settings, scope, muted) : null;
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
                {
                  id: 'inherit',
                  title: 'Default',
                  detail: inherited ? inheritedDetail(inherited) : undefined,
                },
                { id: 'all', title: ALERT_TITLES.all },
                {
                  id: 'mentions',
                  title: ALERT_TITLES.mentions,
                  detail: MENTIONS_DETAIL,
                },
                { id: 'none', title: ALERT_TITLES.none },
              ]}
              onChange={(next) =>
                void configure({
                  action: 'configure',
                  storeId,
                  scope,
                  channel,
                  mode:
                    next === 'inherit'
                      ? null
                      : storedAlertMode(next as AlertChoice),
                })
              }
            />
          </InsetRow>
        </Inset>
      )}
      {session && !session.available ? (
        <p role="alert">
          {noteIcon ? <Icon name={noteIcon} /> : null}
          Desktop alerts are unavailable in this build.
        </p>
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

/**
 * The Mute channel switch. A mute is kept in this device's local settings: it
 * takes the channel's unread count out of its team's and the Chat tab's
 * totals and silences its alerts unless it has its own alert mode. The
 * pinned protocol cannot mute a channel on the server, so it does not reach
 * other devices.
 */
export function ChannelMuteSetting({
  storeId,
  scope,
  channel,
}: {
  storeId: string;
  scope: ChatScope;
  channel: string;
}) {
  const context = useContext(Context);
  // While a change saves there is no session; the projection keeps the mutes
  // last known, so the switch does not flicker.
  const projected = useProjectedMute(context?.service, storeId, channel);
  if (!context) return null;
  const { session, configure } = context;
  const muted = session
    ? session.settings.muted.includes(notificationKey(scope, channel))
    : projected;
  return (
    <label className="chat-info-row chat-info-mute">
      <Icon name="bellOff" className="chat-info-row-icon" />
      <span className="k">Mute channel</span>
      <span className="v">
        <input
          type="checkbox"
          role="switch"
          checked={muted}
          disabled={!session}
          onChange={(event) =>
            void configure({
              action: 'configure',
              storeId,
              scope,
              channel,
              muted: event.target.checked,
            })
          }
        />
      </span>
    </label>
  );
}
