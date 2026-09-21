/** Channel identity, visibility, membership links, and notification settings. */

import { useEffect, useRef, type ReactNode, type RefObject } from 'react';
import { Button, Icon } from '../components';
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
  useEffect(() => {
    // The panel begins translated beyond the window during its entrance.
    // Focusing it must not scroll the shell to that temporary position.
    panel.current?.focus({ preventScroll: true });
  }, []);
  useEffect(() => {
    const dismiss = (event: PointerEvent): void => {
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
  return (
    <aside
      ref={panel}
      className="chat-info"
      aria-label="Channel info"
      tabIndex={-1}
      onKeyDown={(event) => {
        if (event.key === 'Escape' && !event.defaultPrevented) {
          event.preventDefault();
          event.stopPropagation();
          onClose();
        }
      }}
    >
      <div className="chat-info-head">
        <b>Channel info</b>
        <Button
          variant="quiet"
          icon="close"
          aria-label="Close channel info"
          title="Close"
          onClick={() => onClose()}
        />
      </div>
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
      <div className="chat-info-foot">
        <Button
          icon="users"
          title="Manage membership and access to team files and chat"
          onClick={people}
        >
          Manage in Teams
        </Button>
        <Button
          onClick={() =>
            onNavigate({ kind: 'settings', section: 'preferences' })
          }
        >
          Device notification settings
        </Button>
      </div>
    </aside>
  );
}
