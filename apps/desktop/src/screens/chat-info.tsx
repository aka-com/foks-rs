/** Channel identity, visibility, membership links, and notification settings. */

import {
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
  type RefObject,
} from 'react';
import { Icon } from '../components';
import { NotificationSettings } from '../chat/notification-provider';
import { channelLabel } from '../chat/presentation';
import {
  groupDetailFailure,
  hue,
  isMachine,
  partiesOf,
  partyName,
  roleRank,
} from '../model';
import type { AgentSnapshot, TeamStore } from '../model';
import type { ChatChannel, ChatScope } from '../chat-contract';
import type { Location } from '../location';

/** Maximum number of member avatars. */
const MARKS = 3;

/** The window width from which the panel is a column (see `chat.css`). */
const COLUMN_QUERY = '(min-width: 1100px)';

/** Whether the window is wide enough for the panel to be a column. */
function useColumnLayout(): boolean {
  const query = (): MediaQueryList | undefined =>
    typeof window === 'undefined'
      ? undefined
      : window.matchMedia?.(COLUMN_QUERY);
  const [wide, setWide] = useState(() => query()?.matches ?? false);
  useEffect(() => {
    const media = query();
    if (!media) return;
    const changed = (): void => setWide(media.matches);
    changed();
    media.addEventListener?.('change', changed);
    return () => media.removeEventListener?.('change', changed);
  }, []);
  return wide;
}

const TABBABLE =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

export function ChannelInfoPanel({
  snapshot,
  store,
  channel,
  loading = false,
  scope,
  onNavigate,
  onClose,
  toggleRef,
}: {
  snapshot: AgentSnapshot;
  store: TeamStore;
  /** Undefined while the channel inbox is loading. */
  channel?: ChatChannel;
  /** True while awaiting the channel inbox response. */
  loading?: boolean;
  scope?: ChatScope;
  onNavigate: (location: Location) => void;
  onClose: (restoreFocus?: boolean) => void;
  toggleRef?: RefObject<HTMLButtonElement | null>;
}): ReactNode {
  const panel = useRef<HTMLElement>(null);
  // At 1100px and wider the panel is a column beside the conversation: a
  // complementary region in the Tab order after it, closed by the ⓘ toggle
  // or Escape. Narrower, it is drawn over the conversation, so it behaves as
  // a non-modal popover: a dialog that keeps Tab inside it and closes on a
  // click outside.
  const column = useColumnLayout();
  const columnRef = useRef(column);
  columnRef.current = column;
  useEffect(() => {
    // The panel begins translated beyond the window during its entrance.
    // Focusing it must not scroll the shell to that temporary position.
    panel.current?.focus({ preventScroll: true });
  }, []);
  useEffect(() => {
    const dismiss = (event: PointerEvent): void => {
      if (columnRef.current) return;
      const target = event.target;
      if (!(target instanceof Element) || event.button !== 0) return;
      if (
        panel.current?.contains(target) ||
        toggleRef?.current?.contains(target) ||
        target.closest('[role="dialog"], [role="menu"], [role="listbox"]')
      )
        return;
      onClose(false);
    };
    document.addEventListener('pointerdown', dismiss);
    return () => document.removeEventListener('pointerdown', dismiss);
  }, [onClose, toggleRef]);
  const rosterFailure = groupDetailFailure(snapshot, store.id, 'roster');
  const parties = [...partiesOf(snapshot, store.id)].sort(
    (left, right) =>
      roleRank(right.destination_role) - roleRank(left.destination_role),
  );
  const people = (): void => {
    onNavigate({ kind: 'group-settings', ref: store.id, tab: 'people' });
  };
  const onKeyDown = (event: KeyboardEvent<HTMLElement>): void => {
    if (event.key === 'Escape' && !event.defaultPrevented) {
      event.preventDefault();
      event.stopPropagation();
      onClose();
      return;
    }
    if (event.key !== 'Tab' || column || !panel.current) return;
    // Over the conversation, Tab cycles inside the panel instead of walking
    // into the controls it covers.
    const stops = [
      ...panel.current.querySelectorAll<HTMLElement>(TABBABLE),
    ].filter((element) => !element.closest('[hidden], [inert]'));
    if (!stops.length) return;
    const first = stops[0];
    const last = stops[stops.length - 1];
    const active = document.activeElement;
    if (event.shiftKey && (active === first || active === panel.current)) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && active === last) {
      event.preventDefault();
      first.focus();
    }
  };
  return (
    <aside
      ref={panel}
      className={column ? 'chat-info' : 'chat-info overlay'}
      role={column ? 'complementary' : 'dialog'}
      aria-label="Channel info"
      tabIndex={-1}
      onKeyDown={onKeyDown}
    >
      {/* As a column the panel continues the conversation's bar line, as
          the details column does elsewhere; the bar names the channel. */}
      {column ? (
        <div className="chat-info-bar" data-tauri-drag-region="" />
      ) : null}
      {/* Keep team information visible while channel data loads. */}
      <div className="chat-info-identity">
        {channel ? (
          <>
            <span className="chat-info-glyph" aria-hidden="true">
              <Icon name="hash" size={22} />
            </span>
            <b className="chat-info-name">{channelLabel(channel)}</b>
            <p>in {store.name}</p>
            <p className="chat-info-about">
              {channel.description || 'No description.'}
            </p>
          </>
        ) : (
          <p>{loading ? 'Loading channel…' : 'No channel is open.'}</p>
        )}
      </div>
      <div className="chat-info-rows">
        {channel && (
          <div
            className="chat-info-row"
            title={
              channel.admin
                ? 'Hidden from members. Only admins and owners can read or write.'
                : 'Members and administrators can view and post messages.'
            }
          >
            <Icon name="globe" className="chat-info-row-icon" />
            <span className="k">Visibility</span>
            <span className="v">
              {channel.admin ? 'Admins and owners' : 'Everyone on the team'}
            </span>
          </div>
        )}
        <button
          type="button"
          className="chat-info-row chat-info-people"
          // Exclude decorative avatars from the accessible button name.
          title="Manage membership and access to team files and chat"
          onClick={people}
        >
          <Icon name="users" className="chat-info-row-icon" />
          <span className="k">Members</span>
          {!rosterFailure && (
            <span className="v">
              <span className="chat-info-marks" aria-hidden="true">
                {parties.slice(0, MARKS).map((party) => {
                  // Match the avatars in the team member list.
                  const machine = isMachine(party);
                  const name = partyName(party);
                  return (
                    <span
                      className="chat-info-mark"
                      key={party.party_id_hex}
                      style={{
                        background: machine ? 'var(--c-none)' : hue(name),
                      }}
                    >
                      {machine ? (
                        <Icon name="terminal" />
                      ) : (
                        name.slice(0, 1).toUpperCase()
                      )}
                    </span>
                  );
                })}
              </span>
              {parties.length}
            </span>
          )}
          <Icon name="chevronRight" className="chat-info-go" />
        </button>
        {channel && (
          <div className="chat-info-row chat-info-alerts">
            <Icon name="bell" className="chat-info-row-icon" />
            <NotificationSettings
              storeId={store.id}
              scope={scope}
              channel={channel.id}
              noteIcon="alert"
            />
          </div>
        )}
      </div>
      {rosterFailure && (
        <p className="chat-info-note" role="alert">
          {rosterFailure.message}
        </p>
      )}
      {/* Membership is the Members row above; the device-wide alert
          settings are a quiet link under the rows rather than a foot of
          full-width buttons. */}
      <div className="chat-info-foot">
        <button
          type="button"
          className="lnk"
          onClick={() =>
            onNavigate({ kind: 'settings', section: 'preferences' })
          }
        >
          Device notification settings
        </button>
      </div>
    </aside>
  );
}
