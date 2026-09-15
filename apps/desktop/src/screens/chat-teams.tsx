/**
 * The Chat tab's team column.
 *
 * Every named team whose server offers chat is a heading button with its
 * unread badge, beside the buttons that act on that team; exactly one of them
 * is the current team — `aria-current`, because the row selects rather than
 * folds — and lists its channels. Collapsed teams carry only the facts the
 * rail already reads — `useSidebarInbox()` plus `teamUnread` — so no inbox is
 * mounted for them. Named teams whose server offers no chat sit dimmed at the
 * foot under "No chat"; shares are not listed, because chat lives in a named
 * team. The column belongs to the tab and outlives a team switch.
 */

import { useState } from 'react';
import type { ReactNode } from 'react';
import { Button, Icon, SectionLabel } from '../components';
import {
  chatAvailable,
  shortId,
  storeDescription,
  storeNavigationOrder,
} from '../model';
import type {
  AgentSnapshot,
  AvailabilityOptions,
  Server,
  StoreRef,
  TeamStore,
} from '../model';
import { channelTitle, listChannels } from '../chat/presentation';
import type { ListedChannel } from '../chat/presentation';
import { useSidebarInbox } from '../chat/inbox-provider';
import { teamUnread } from '../chat/unread';
import { GroupMark } from './groups-screen';

/** The server a store belongs to, whatever its state. */
function serverFor(
  snapshot: AgentSnapshot,
  store: TeamStore,
): Server | undefined {
  return snapshot.servers.find((server) => server.id === store.server);
}

/**
 * The named teams the column lists: active teams on a server that offers chat.
 * A lapsed check-in keeps a team listed and shows it as locked, because the
 * team still has chat — this Mac just cannot reach it right now.
 */
export function chatTeams(snapshot: AgentSnapshot): TeamStore[] {
  return storeNavigationOrder(snapshot).filter(
    (store): store is TeamStore =>
      store.kind === 'team' &&
      store.team_kind === 'named' &&
      store.active !== false &&
      serverFor(snapshot, store)?.capabilities.chat === true,
  );
}

/**
 * Why a named team is not in the column: the one condition `chatTeams`
 * filtered it out on.
 */
export function noChatReason(
  snapshot: AgentSnapshot,
  store: TeamStore,
): string {
  const server = serverFor(snapshot, store);
  if (store.active === false) return 'Setup incomplete';
  if (!server) return 'Server unavailable';
  return `Chat not offered on ${server.name}`;
}

/** Named teams with no chat at all, listed under "No chat" with the reason. */
function noChatTeams(snapshot: AgentSnapshot): TeamStore[] {
  const listed = new Set(chatTeams(snapshot).map((store) => store.id));
  return storeNavigationOrder(snapshot).filter(
    (store): store is TeamStore =>
      store.kind === 'team' &&
      store.team_kind === 'named' &&
      !listed.has(store.id),
  );
}

/** The team the tab opens when the location names none. */
export function firstChatTeam(
  snapshot: AgentSnapshot,
  options: AvailabilityOptions = {},
): TeamStore | undefined {
  const listed = chatTeams(snapshot);
  return (
    listed.find((store) => chatAvailable(snapshot, store, options)) ?? listed[0]
  );
}

/**
 * "3 channels · 2 unread", or what is known while the inbox is still loading.
 * An inbox that failed says so rather than loading for ever.
 */
function teamSummary(
  channels: number | null,
  unread: string | null,
  failure: string,
): string {
  if (failure) return failure;
  if (channels === null) return 'Loading channels…';
  const head = `${channels} ${channels === 1 ? 'channel' : 'channels'}`;
  return unread ? `${head} · ${unread} unread` : head;
}

export interface ChatTeamColumnProps {
  snapshot: AgentSnapshot;
  /** The team whose inbox is mounted, when there is one. */
  selected?: StoreRef;
  /** The open team's channels, from the mounted conversation. */
  channels?: readonly ListedChannel[];
  /** The open channel. */
  activeChannel?: string;
  /** The open team's channels that failed verification. */
  blockedChannels?: ReadonlySet<string>;
  /** The local actor, so an own preview reads "You". */
  actor?: string | null;
  /** Party names for preview senders. */
  senderNames?: ReadonlyMap<string, string>;
  /** The open team's inbox is still on its first synchronization. */
  loading?: boolean;
  /** The open team's server is locked, so its channels cannot be opened. */
  locked?: boolean;
  /** The clock the tab decides availability on, so the column shares it. */
  accessOptions?: AvailabilityOptions;
  onSelectTeam: (ref: StoreRef) => void;
  onOpenChannel: (channel: string) => void;
  onNewChannel: () => void;
  onSettings: (ref: StoreRef) => void;
  onCreateTeam: () => void;
}

export function ChatTeamColumn({
  snapshot,
  selected,
  channels = [],
  activeChannel,
  blockedChannels,
  actor = null,
  senderNames,
  loading = false,
  locked = false,
  accessOptions = {},
  onSelectTeam,
  onOpenChannel,
  onNewChannel,
  onSettings,
  onCreateTeam,
}: ChatTeamColumnProps): ReactNode {
  const inbox = useSidebarInbox();
  const teams = chatTeams(snapshot);
  const withoutChat = noChatTeams(snapshot);
  const open = teams.find((store) => store.id === selected);
  // The column outlives a team switch, so a filter typed for one team is
  // cleared when another opens.
  const [filter, setFilter] = useState('');
  const [filtered, setFiltered] = useState(open?.id);
  if (filtered !== open?.id) {
    setFiltered(open?.id);
    if (filter) setFilter('');
  }
  const query = filter.trim().toLowerCase();
  const shown = open
    ? channels.filter(
        ({ channel }) =>
          !query || channelTitle(channel).toLowerCase().includes(query),
      )
    : [];
  return (
    <aside className="chat-inbox" aria-label="Chat inbox">
      <div className="chat-inbox-top">
        <label className="search">
          <Icon name="search" />
          <input
            type="text"
            value={filter}
            disabled={!open}
            placeholder={open ? `Search ${open.name}` : 'Search'}
            aria-label={
              open ? `Search channels in ${open.name}` : 'Search channels'
            }
            autoComplete="off"
            onChange={(event) => setFilter(event.target.value)}
          />
        </label>
      </div>
      <div className="chat-inbox-scroll">
        {teams.length ? (
          <SectionLabel>Teams</SectionLabel>
        ) : (
          <p className="chat-quiet chat-inbox-empty">
            No team on this Mac has chat.
          </p>
        )}
        {teams.map((store) => {
          const expanded = store.id === open?.id;
          const reachable = chatAvailable(snapshot, store, accessOptions);
          const entry = inbox.get(store.id);
          // An inbox that failed says so here as well: the column and the pane
          // cannot disagree about whether channels are still on their way.
          const failure =
            entry &&
            (entry.error ||
              entry.state === 'unavailable' ||
              entry.state === 'blocked')
              ? entry.error || 'Channels unavailable'
              : '';
          const unread = reachable ? teamUnread(entry) : null;
          // An unreachable team says what is wrong, in the words the rest of
          // the shell uses for that store.
          const unavailable = reachable
            ? ''
            : storeDescription(snapshot, store, accessOptions);
          const badge = reachable
            ? unread
            : { label: '!', description: unavailable };
          // The channels of a team are its conversations and the channels that
          // have none yet, which is what the expanded list draws.
          const counted = entry?.data
            ? listChannels(entry.data.channels, entry.data.conversations).length
            : null;
          const server = serverFor(snapshot, store);
          // Only a plain count belongs in a sentence: "…", "!", "?", "3+" and
          // "3·" are states, and the badge's own description says them.
          const countable =
            unread && /^\d+$/.test(unread.label) ? unread.label : null;
          return (
            <div
              className={expanded ? 'chat-team open' : 'chat-team'}
              key={store.id}
            >
              <button
                type="button"
                className={expanded ? 'chat-team-head open' : 'chat-team-head'}
                // The row selects a team; it does not fold one away, so it is
                // the current team rather than an expanded disclosure.
                aria-current={expanded ? 'true' : undefined}
                title={[
                  `${store.name} · ${server?.name ?? store.server}`,
                  badge?.description,
                ]
                  .filter(Boolean)
                  .join(' · ')}
                onClick={() => {
                  if (!expanded) onSelectTeam(store.id);
                }}
              >
                {/* One group, one mark: the column, the Teams row and the
                    group's own page draw the same initial over the same
                    colour. */}
                <GroupMark store={store} size="sm" />
                <span className="t">
                  <b>{store.name}</b>
                  <small>
                    {expanded
                      ? (server?.name ?? store.server)
                      : unavailable || teamSummary(counted, countable, failure)}
                  </small>
                </span>
                {badge && (
                  <span className="chat-unread" aria-label={badge.description}>
                    {badge.label}
                  </span>
                )}
              </button>
              {expanded && reachable && (
                <Button
                  variant="quiet"
                  icon="plus"
                  aria-label="New channel"
                  title={`New channel in ${store.name}`}
                  onClick={() => onNewChannel()}
                />
              )}
              <Button
                variant="quiet"
                icon="gear"
                aria-label={`Group settings for ${store.name}`}
                title={`Group settings for ${store.name}`}
                onClick={() => onSettings(store.id)}
              />
              {expanded && loading && !channels.length && (
                <p className="chat-quiet chat-loadrow" role="status">
                  Loading {store.name}…
                </p>
              )}
              {expanded && (
                <div className="chat-channel-list">
                  {shown.map(({ channel, conversation }) => {
                    const active = channel.id === activeChannel;
                    const count =
                      conversation && BigInt(conversation.unread) > 0n
                        ? conversation.unread
                        : null;
                    const meta = blockedChannels?.has(channel.id)
                      ? 'Verification stopped'
                      : [
                          !channel.readable ? 'Restricted' : '',
                          conversation?.muted ? 'Muted' : '',
                        ]
                          .filter(Boolean)
                          .join(' · ');
                    return (
                      <button
                        type="button"
                        key={channel.id}
                        className={[
                          'chat-channel',
                          active ? 'on' : '',
                          count ? 'unread' : '',
                          locked ? 'off' : '',
                        ]
                          .filter(Boolean)
                          .join(' ')}
                        aria-current={active ? 'page' : undefined}
                        disabled={locked}
                        onClick={() => onOpenChannel(channel.id)}
                      >
                        <span className="t">
                          <span className="n">{channelTitle(channel)}</span>
                          {meta && <small>{meta}</small>}
                          {conversation?.preview && (
                            <small>
                              {conversation.preview.sender === actor
                                ? 'You'
                                : conversation.preview.sender
                                  ? (senderNames?.get(
                                      conversation.preview.sender,
                                    ) ?? shortId(conversation.preview.sender))
                                  : 'Team member'}
                              {': '}
                              {conversation.preview.content.kind === 'text'
                                ? conversation.preview.content.text
                                : 'Unsupported message'}
                            </small>
                          )}
                        </span>
                        {channel.admin && (
                          <span
                            className="lock"
                            title="Admins and owners only"
                            aria-label="Admins and owners only"
                          >
                            <Icon name="key" size={13} />
                          </span>
                        )}
                        {count && !locked && (
                          <span
                            className={
                              conversation?.muted
                                ? 'chat-unread muted'
                                : 'chat-unread'
                            }
                            aria-label={`${count} unread`}
                          >
                            {count}
                          </span>
                        )}
                      </button>
                    );
                  })}
                  {!shown.length && channels.length > 0 && (
                    <p className="chat-quiet">
                      No channel in {store.name} matches “{filter}”.
                    </p>
                  )}
                  {!channels.length &&
                    !locked &&
                    (failure ? (
                      // The open team's pane states the failure and carries
                      // the retry; the column says only that no channel list
                      // is coming, rather than printing the same line twice.
                      <p className="chat-quiet">Channels unavailable.</p>
                    ) : (
                      !loading && (
                        <p className="chat-quiet">
                          No channels yet. Create the first one to start
                          talking.
                        </p>
                      )
                    ))}
                </div>
              )}
            </div>
          );
        })}
        {withoutChat.length > 0 && (
          <>
            <SectionLabel className="chat-nochat-label">No chat</SectionLabel>
            {withoutChat.map((store) => {
              const reason = noChatReason(snapshot, store);
              return (
                <div
                  className="chat-team-head off"
                  key={store.id}
                  title={`${store.name} · ${reason}`}
                >
                  <GroupMark store={store} size="sm" />
                  <span className="t">
                    <b>{store.name}</b>
                    <small>{reason}</small>
                  </span>
                </div>
              );
            })}
          </>
        )}
      </div>
      <div className="chat-inbox-foot">
        <Button onClick={onCreateTeam}>Create or join a team</Button>
      </div>
    </aside>
  );
}
