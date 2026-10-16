/**
 * Channel identity, who can read and post, the readers, and notification
 * settings.
 */

import {
  useEffect,
  useId,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from 'react';
import { Button, Chip, Icon } from '../../components';
import {
  ChannelMuteSetting,
  NotificationSettings,
  useLocalAlerts,
} from '../../chat/notification-provider';
import { teamMuted } from '../../chat/local-contract';
import { channelLabel } from '../../chat/presentation';
import {
  channelAudience,
  channelRoles,
  everyonePosts,
  everyoneReads,
  roleAudience,
  type ChannelReader,
} from '../../chat/audience';
import {
  groupDetailFailure,
  hue,
  isMachine,
  parseRole,
  partyName,
  plural,
  roleName,
} from '../../model';
import type { AgentSnapshot, Party, TeamStore } from '../../model';
import type { ChatChannel, ChatScope } from '../../chat-contract';
import type { Location } from '../../location';

/** Maximum number of member avatars. */
const MARKS = 3;
/** Up to this many team users who cannot read are named; more are counted. */
const NAMED_OTHERS = 5;

/** A party's avatar, matching the avatars in the team member list. */
function PartyMark({ party }: { party: Party }): ReactNode {
  const machine = isMachine(party);
  const name = partyName(party);
  return (
    <span
      className="chat-info-mark"
      style={{ background: machine ? 'var(--c-none)' : hue(name) }}
    >
      {machine ? <Icon name="terminal" /> : name.slice(0, 1).toUpperCase()}
    </span>
  );
}

/** One reader: avatar, name, role, and whether the role can post. */
function ReaderRow({ reader }: { reader: ChannelReader }): ReactNode {
  const { party, post } = reader;
  const role = parseRole(party.destination_role);
  return (
    <li className="chat-info-row chat-info-reader">
      <span aria-hidden="true">
        <PartyMark party={party} />
      </span>
      <span className="chat-info-who">
        <b>
          <span>{partyName(party)}</span>
          {party.label === 'you' && <Chip tone="you">You</Chip>}
        </b>
        <small>
          {role ? `${roleName(role)} · ` : ''}
          {post ? 'can post' : <span className="ro">read only</span>}
        </small>
      </span>
    </li>
  );
}

/** "a", "a and b", "a, b and c". */
function nameList(names: readonly string[]): string {
  return names.length < 2
    ? names.join('')
    : `${names.slice(0, -1).join(', ')} and ${names.at(-1)}`;
}

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
  // A failed roster load leaves whatever parties were held; no reader list is
  // drawn from them.
  const audience =
    channel && !rosterFailure
      ? channelAudience(snapshot, store.id, channel)
      : undefined;
  const roles = channel && channelRoles(channel);
  // A channel every team user can read folds its reader list behind one row.
  const [everyoneOpen, setEveryoneOpen] = useState(false);
  const everyoneList = useId();
  const muteNote = useId();
  const localSettings = useLocalAlerts()?.session?.settings;
  const muteFromTeam =
    !!scope && !!localSettings && teamMuted(localSettings, scope);
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
      {channel && (
        <>
          <h3 className="chat-info-sec">Access</h3>
          <div className="chat-info-rows">
            <div className="chat-info-row">
              <Icon name="eye" className="chat-info-row-icon" />
              <span className="k">Who can read</span>
              <span className="v">
                {audience
                  ? roleAudience(audience.read, everyoneReads(audience))
                  : roles
                    ? roleAudience(roles.read)
                    : channel.read_role}
              </span>
            </div>
            <div className="chat-info-row">
              <Icon name="pencil" className="chat-info-row-icon" />
              <span className="k">Who can post</span>
              <span className="v">
                {audience
                  ? roleAudience(audience.write, everyonePosts(audience))
                  : roles
                    ? roleAudience(roles.write)
                    : channel.write_role}
              </span>
            </div>
          </div>
        </>
      )}
      {audience && (
        <>
          <h3 className="chat-info-sec">
            Readers <span className="n">{audience.readers.length}</span>
          </h3>
          {everyoneReads(audience) ? (
            <>
              <div className="chat-info-rows">
                <button
                  type="button"
                  className="chat-info-row chat-info-people"
                  aria-expanded={everyoneOpen}
                  aria-controls={everyoneList}
                  onClick={() => setEveryoneOpen((open) => !open)}
                >
                  <Icon name="users" className="chat-info-row-icon" />
                  <span className="k">All members</span>
                  <span className="v">
                    <span className="chat-info-marks" aria-hidden="true">
                      {audience.readers.slice(0, MARKS).map(({ party }) => (
                        <PartyMark key={party.party_id_hex} party={party} />
                      ))}
                    </span>
                    {audience.readers.length}
                  </span>
                  <Icon
                    name={everyoneOpen ? 'chevronDown' : 'chevronRight'}
                    className="chat-info-go"
                  />
                </button>
              </div>
              <ul
                id={everyoneList}
                className="chat-info-rows chat-info-readers"
                aria-label="Readers"
                hidden={!everyoneOpen}
              >
                {audience.readers.map((reader) => (
                  <ReaderRow key={reader.party.party_id_hex} reader={reader} />
                ))}
              </ul>
            </>
          ) : (
            <>
              <ul
                className="chat-info-rows chat-info-readers"
                aria-label="Readers"
              >
                {audience.readers.map((reader) => (
                  <ReaderRow key={reader.party.party_id_hex} reader={reader} />
                ))}
              </ul>
              <p className="chat-info-aside">
                {plural(audience.others.length, 'other team member')} cannot
                read this channel
                {audience.others.length <= NAMED_OTHERS
                  ? `: ${nameList(audience.others.map(partyName))}.`
                  : '.'}
              </p>
            </>
          )}
        </>
      )}
      {channel && (
        <>
          <h3 className="chat-info-sec">Notifications</h3>
          <div className="chat-info-rows">
            <div className="chat-info-row chat-info-alerts">
              <Icon name="bell" className="chat-info-row-icon" />
              <NotificationSettings
                storeId={store.id}
                scope={scope}
                channel={channel.id}
                noteIcon="alert"
              />
            </div>
            {scope && (
              <ChannelMuteSetting
                storeId={store.id}
                scope={scope}
                channel={channel.id}
                describedBy={muteNote}
              />
            )}
          </div>
          {scope && (
            <p id={muteNote} className="chat-info-aside chat-info-mute-note">
              Mute applies to this device only.{' '}
              {muteFromTeam &&
                `All ${store.name} channels are muted from the team menu. `}
              A muted channel is left out of the team and Chat unread counts,
              and raises no alerts unless the channel has its own Alerts
              setting.
            </p>
          )}
        </>
      )}
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
