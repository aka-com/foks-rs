/**
 * The Chat tab's inbox column: every team with chat, at once.
 *
 * Every team is a heading with its `#channels` indented under it, the general
 * channel included, so a team's row means the same thing whether it has one
 * channel or ten: clicking it folds or unfolds the channel list, and clicking a
 * channel opens that channel. The fold state is local to the column and keyed
 * by team id — it does not persist. A collapsed team keeps its unread total on
 * the heading, so folding the channel list away loses no information about
 * what is unread. Each channel row carries its own unread count and, when it
 * has nothing else to say, the last message and when it arrived.
 * The rows are built from `useSidebarInbox()` — the per-team projections
 * `ChatInboxService` already keeps for the rail's badge — so a team is listed,
 * previewed and counted without a conversation being mounted for it. Named
 * teams whose server offers no chat sit dimmed at the foot under "Chat
 * unavailable"; shares are not listed, because chat lives in a named team. The
 * column belongs to the tab and outlives a team switch.
 */

import { useId, useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import { Button, Icon, SectionLabel } from '../components';
import {
  chatAvailable,
  serverDisplayName,
  teamCaption,
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
import {
  channelMeta,
  channelTitle,
  listChannels,
  partyNames,
  previewLine,
  previewTime,
} from '../chat/presentation';
import type { ListedChannel } from '../chat/presentation';
import { useSidebarInbox } from '../chat/inbox-provider';
import type { TeamInbox } from '../chat/inbox-service';
import { teamUnread } from '../chat/unread';
import { GroupMark } from './group-mark';

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
      serverFor(snapshot, store)?.services.chat === true,
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
  if (store.active === false) return 'Finish setup in Teams';
  if (!server) return 'Server unavailable';
  if (server.services.chat === null)
    return 'Chat support has not been determined';
  return `Chat is not enabled on ${serverDisplayName(server)}`;
}

/** Named teams with no chat at all, listed under "Chat unavailable" with the reason. */
export function noChatTeams(snapshot: AgentSnapshot): TeamStore[] {
  const listed = new Set(chatTeams(snapshot).map((store) => store.id));
  return storeNavigationOrder(snapshot).filter(
    (store): store is TeamStore =>
      store.kind === 'team' &&
      store.team_kind === 'named' &&
      !listed.has(store.id),
  );
}

export function pickerConversations(
  snapshot: AgentSnapshot,
  inbox: ReadonlyMap<string, TeamInbox>,
  query: string,
  options: AvailabilityOptions = {},
) {
  const search = query.trim().toLowerCase();
  return chatTeams(snapshot).flatMap((store) => {
    const entry = inbox.get(store.id);
    const available =
      chatAvailable(snapshot, store, options) && entry?.state !== 'blocked';
    return (
      entry?.data
        ? listChannels(entry.data.channels, entry.data.conversations)
        : []
    )
      .filter(
        ({ channel }) =>
          !search ||
          `${store.name} ${channelTitle(channel)} ${channel.description ?? ''}`
            .toLowerCase()
            .includes(search),
      )
      .map((option) => ({ store, entry, option, available }));
  });
}

/** The team the tab falls back to when no conversation has any activity. */
export function firstChatTeam(
  snapshot: AgentSnapshot,
  options: AvailabilityOptions = {},
): TeamStore | undefined {
  const listed = chatTeams(snapshot);
  return (
    listed.find((store) => chatAvailable(snapshot, store, options)) ?? listed[0]
  );
}

/** Whether the team inbox is awaiting its initial response. */
function firstSynchronization(entry: TeamInbox | undefined): boolean {
  return !entry || (entry.state === 'loading' && !entry.data && !entry.error);
}

/**
 * The conversation `{kind:'chat'}` with no team opens: the one with the most
 * recent message across every team this Mac can reach, else the first team
 * that has chat. `'pending'` means a reachable team the service has actually
 * started on is still on its first synchronization and nothing else has
 * answered yet, so the answer is not knowable yet and the tab waits rather
 * than opening a team it would have to leave.
 *
 * The wait is bounded twice over: it ends as soon as one reachable team has
 * answered, and a team missing from an inbox that already holds entries is
 * never waited on. The service publishes an entry for every team it keeps in
 * one pass, so an empty map means it has not been asked yet, while a team
 * absent from a filled one is a team it does not keep — waiting on that team
 * would be waiting for an entry that never arrives. The tab keeps re-resolving
 * until selection becomes final, allowing a more recent conversation to
 * replace the first response.
 */
export function openingConversation(
  snapshot: AgentSnapshot,
  inbox: ReadonlyMap<string, TeamInbox>,
  options: AvailabilityOptions = {},
): { ref: StoreRef; channel?: string } | 'pending' | undefined {
  const teams = chatTeams(snapshot);
  if (!teams.length) return undefined;
  const reachable = teams.filter((store) =>
    chatAvailable(snapshot, store, options),
  );
  const entries = reachable.map((store) => inbox.get(store.id));
  const answered = entries.some((entry) => entry?.data || entry?.error);
  const asked = inbox.size > 0;
  const waiting = entries.some((entry) =>
    entry ? firstSynchronization(entry) : !asked,
  );
  if (!answered && waiting) return 'pending';
  let best: { ref: StoreRef; channel: string; at: bigint } | undefined;
  for (const store of reachable)
    for (const conversation of inbox.get(store.id)?.data?.conversations ?? []) {
      if (conversation.hidden || !conversation.preview) continue;
      const at = BigInt(conversation.preview.insert_time);
      // Ties keep navigation order: the earlier team stays the one that opens.
      if (!best || at > best.at)
        best = { ref: store.id, channel: conversation.channel.id, at };
    }
  if (best) return { ref: best.ref, channel: best.channel };
  const fallback = firstChatTeam(snapshot, options);
  return fallback ? { ref: fallback.id } : undefined;
}

/** One team as the column draws it, whether as a row or as a heading. */
interface TeamRow {
  store: TeamStore;
  entry: TeamInbox | undefined;
  /** This Mac can open the team's chat right now. */
  reachable: boolean;
  /** The one line a row states instead of a preview: a reason or a failure. */
  status: string;
  /**
   * What a synchronization that succeeded could not finish, said beside the
   * preview rather than in place of it.
   */
  note: string;
  /** `undefined` while the team's channel list has not arrived. */
  channels: ListedChannel[] | undefined;
  badge: { label: string; description: string } | null;
  /** The team's people, resolved once per team rather than once per row. */
  names: ReadonlyMap<string, string>;
  server: string;
}

/** A badge that is a bare number, as opposed to "…", "!", "3+" or "3·". */
function plainCount(label: string): boolean {
  return /^\d+$/.test(label);
}

function teamRow(
  snapshot: AgentSnapshot,
  inbox: ReadonlyMap<string, TeamInbox>,
  store: TeamStore,
  options: AvailabilityOptions,
  /** The team whose conversation is mounted beside the column. */
  open: boolean,
): TeamRow {
  const entry = inbox.get(store.id);
  const reachable = chatAvailable(snapshot, store, options);
  // An inbox that failed says so here as well: the column and the pane cannot
  // disagree about whether channels are still on their way. The open team's
  // pane states the failure itself and carries the retry, so its row says only
  // that no channel list is coming rather than printing the same line twice.
  // A synchronization that succeeded but could not finish everything — a read
  // status that will retry, a preview it could not fetch — carries its error
  // alongside the data it did bring, so it is a note rather than a failure.
  const failed =
    entry !== undefined &&
    (entry.state === 'unavailable' ||
      entry.state === 'blocked' ||
      (Boolean(entry.error) && !entry.data));
  const failure = failed
    ? open
      ? 'Channels unavailable'
      : entry.error || 'Channels unavailable'
    : '';
  // An unreachable team says what is wrong, in the words the rest of the shell
  // uses for that store.
  const unavailable = reachable
    ? ''
    : storeDescription(snapshot, store, options);
  const loading = firstSynchronization(entry);
  const unread = reachable && !loading ? teamUnread(entry) : null;
  return {
    store,
    entry,
    reachable,
    status: unavailable || failure || (loading ? 'Loading channels…' : ''),
    note: !failed && reachable && entry?.data ? entry.error || entry.note : '',
    channels: entry?.data
      ? listChannels(entry.data.channels, entry.data.conversations)
      : undefined,
    badge: reachable ? unread : { label: '!', description: unavailable },
    names: partyNames(snapshot, store.id),
    server: teamCaption(snapshot, store, { kind: false }),
  };
}

/** A channel's own unread count, or `null` when it has none. */
function channelCount(listed: ListedChannel | undefined): string | null {
  const conversation = listed?.conversation;
  return conversation && BigInt(conversation.unread) > 0n
    ? conversation.unread
    : null;
}

export interface ChatTeamColumnProps {
  snapshot: AgentSnapshot;
  /** The team whose conversation is mounted, when there is one. */
  selected?: StoreRef;
  /** The open channel. */
  activeChannel?: string;
  /** The clock the tab decides availability on, so the column shares it. */
  accessOptions?: AvailabilityOptions;
  onOpen: (ref: StoreRef, channel?: string) => void;
  onNewChat: () => void;
  /** Opens a team's page on the Teams tab, where its setup is finished. */
  onTeams: (ref: StoreRef) => void;
}

export function ChatTeamColumn({
  snapshot,
  selected,
  activeChannel,
  accessOptions = {},
  onOpen,
  onNewChat,
  onTeams,
}: ChatTeamColumnProps): ReactNode {
  const inbox = useSidebarInbox();
  // Which teams have their channel list folded away, by team id.
  // This is local to the column and does not persist: a team reopens expanded
  // the next time the column mounts.
  const [collapsedTeams, setCollapsedTeams] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const toggleCollapsed = (id: string) =>
    setCollapsedTeams((prior) => {
      const next = new Set(prior);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  // The column searches names, not messages: the agent has no message index,
  // so a query that matched text would be a promise the client cannot keep.
  const [filter, setFilter] = useState('');
  const query = filter.trim().toLowerCase();
  // The clock is the tab's, but the object it arrives in is rebuilt every
  // render, so the memo is keyed on the moment rather than on its identity —
  // and on the second rather than on the fraction of a millisecond the tab
  // reads, which no two renders share and which would make the memo a cost
  // with no hit. Availability changes on the second; a check-in that lapses
  // between two ticks is seen on the next one.
  const { nowSeconds, agentReady, catalogReady } = accessOptions;
  const second = nowSeconds === undefined ? undefined : Math.floor(nowSeconds);
  const now = nowSeconds === undefined ? Date.now() : nowSeconds * 1000;
  // Every row reads the whole snapshot and the whole inbox map, so building
  // them is worth doing once per change rather than once per keystroke in the
  // search field.
  const { teams, withoutChat, rows } = useMemo(() => {
    const options: AvailabilityOptions = {
      nowSeconds: second,
      agentReady,
      catalogReady,
    };
    const listedTeams = chatTeams(snapshot);
    return {
      teams: listedTeams,
      withoutChat: noChatTeams(snapshot),
      rows: listedTeams.map((store) =>
        teamRow(snapshot, inbox, store, options, store.id === selected),
      ),
    };
  }, [snapshot, inbox, second, agentReady, catalogReady, selected]);
  const dimmed = withoutChat.filter(
    (store) => !query || store.name.toLowerCase().includes(query),
  );
  let listed = 0;
  const teamRows: ReactNode[] = [];
  rows.forEach((row) => {
    const named = row.store.name.toLowerCase().includes(query);
    const matching = (row.channels ?? []).filter(
      ({ channel }) =>
        !query || named || channelTitle(channel).toLowerCase().includes(query),
    );
    if (query && !named && !matching.length) return;
    listed += 1;
    teamRows.push(
      <TeamHeading
        key={row.store.id}
        row={row}
        channels={matching}
        now={now}
        selected={selected}
        activeChannel={row.store.id === selected ? activeChannel : undefined}
        // A search in progress overrides a fold: a channel the query
        // matched inside a collapsed team has to be seen to explain why its
        // team matched at all. The fold itself is untouched underneath, and
        // returns as soon as the query is cleared.
        collapsed={query ? false : collapsedTeams.has(row.store.id)}
        onToggleCollapse={() => toggleCollapsed(row.store.id)}
        onOpen={onOpen}
      />,
    );
  });
  return (
    <aside className="chat-inbox" aria-label="Chat inbox">
      <div className="chat-inbox-top">
        {/* The field names itself, so there is no label to wrap it in: an empty
            `<label>` would be a label with nothing in it. */}
        <div className="search">
          <Icon name="search" />
          <input
            type="search"
            value={filter}
            // The "No chat" teams are part of the column and are searched with
            // the rest of it, so the field is live whenever the column lists
            // anything at all.
            disabled={!teams.length && !withoutChat.length}
            placeholder="Search"
            aria-label="Search teams and channels"
            autoComplete="off"
            onChange={(event) => setFilter(event.target.value)}
          />
        </div>
        <Button
          variant="primary"
          icon="plus"
          // The sheet this opens creates a channel; existing conversations
          // open from the column below it, so the button names what it does.
          aria-label="New channel"
          disabled={!teams.length}
          title={
            teams.length
              ? 'New channel'
              : 'Chat requires a team on a server with chat enabled'
          }
          onClick={onNewChat}
        />
      </div>
      {/* Screen reader status announcement for search filtering results.
          The element is rendered persistently to ensure aria-live announcements
          fire. */}
      <p className="offscreen" role="status">
        {query
          ? `${listed + dimmed.length} of ${teams.length + withoutChat.length} teams match ${filter}.`
          : ''}
      </p>
      <div className="chat-inbox-scroll">
        {/* The heading draws only once there is a team under it: a search
            that clears every team drops the heading with it. */}
        {teamRows.length > 0 && <SectionLabel>Teams</SectionLabel>}
        {teamRows}
        {query &&
          !listed &&
          !dimmed.length &&
          teams.length + withoutChat.length > 0 && (
            <p className="chat-quiet chat-inbox-empty">
              No team or channel matches “{filter}”.
            </p>
          )}
        {dimmed.length > 0 && (
          <>
            <SectionLabel className="chat-nochat-label">
              Chat unavailable
            </SectionLabel>
            {dimmed.map((store) => {
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
                    <small>
                      {store.active === false ? (
                        <>
                          Finish setup in{' '}
                          <button
                            type="button"
                            className="lnk"
                            onClick={() => onTeams(store.id)}
                          >
                            Teams
                          </button>
                        </>
                      ) : (
                        reason
                      )}
                    </small>
                  </span>
                </div>
              );
            })}
          </>
        )}
      </div>
    </aside>
  );
}

/**
 * The sum of what a team's own channel rows are already carrying, on the same
 * terms `teamUnread` counts by: a hidden or muted channel does not contribute,
 * so folding a team with only a muted channel unread does not manufacture an
 * unread signal collapsing it would otherwise not have shown.
 */
function channelUnreadTotal(channels: readonly ListedChannel[]): number {
  return channels.reduce((sum, listed) => {
    const { conversation } = listed;
    if (!conversation || conversation.hidden || conversation.muted) return sum;
    const count = channelCount(listed);
    return count ? sum + Number(count) : sum;
  }, 0);
}

/** A team: a heading that folds its channels, and the channels under it. */
function TeamHeading({
  row,
  channels,
  now,
  selected,
  activeChannel,
  collapsed,
  onToggleCollapse,
  onOpen,
}: {
  row: TeamRow;
  channels: readonly ListedChannel[];
  now: number;
  /** The team whose conversation is mounted beside the column. */
  selected?: StoreRef;
  activeChannel?: string;
  /** The channel list is folded away; local to the column, keyed by team id. */
  collapsed: boolean;
  onToggleCollapse: () => void;
  onOpen: (ref: StoreRef, channel?: string) => void;
}): ReactNode {
  // A heading's own count is normally the sum of the counts already drawn
  // beside its channels, so only what the channels cannot say is drawn here:
  // that the team cannot be reached, that its count is degraded, that it is
  // going stale. Collapsed, the channels are not drawn at all, so the heading
  // carries their total instead — nothing is lost by folding the list away.
  // The channel list this team's row expands, named so the row can point at
  // it rather than leaving the relationship to visual order alone.
  const channelListId = useId();
  const state = row.badge && !plainCount(row.badge.label) ? row.badge : null;
  const collapsedTotal = collapsed ? channelUnreadTotal(channels) : 0;
  const badge =
    state ??
    (collapsedTotal > 0
      ? {
          label: String(collapsedTotal),
          description: `${collapsedTotal} unread`,
        }
      : null);
  // A team whose channel list has arrived empty says so inside its disclosure.
  // The general channel of an otherwise empty team says the same of its
  // messages, so a fresh team is not two bare names. Unknown or inaccessible
  // channel lists are not treated as empty.
  const empty =
    row.reachable && row.entry?.state === 'ready' && row.channels?.length === 0;
  const lone =
    row.channels?.length === 1 && !row.channels[0].channel.name
      ? row.channels[0]
      : undefined;
  // A known empty list folds its placeholder row just like a channel list.
  // A team whose list has not arrived opens its own pane, which states
  // the reason: loading, locked, or unavailable.
  const foldable = channels.length > 0 || empty;
  const open = activeChannel !== undefined || row.store.id === selected;
  return (
    <div className={collapsed ? 'chat-team collapsed' : 'chat-team'}>
      {/* The whole row is the fold control: it has no other click to collide
          with, since opening a channel is a click on the channel's own row. */}
      <button
        type="button"
        className={foldable || !open ? 'chat-team-head' : 'chat-team-head on'}
        aria-expanded={foldable ? !collapsed : undefined}
        aria-controls={foldable ? channelListId : undefined}
        aria-current={!foldable && open ? 'page' : undefined}
        aria-label={
          !foldable
            ? undefined
            : collapsed
              ? `Expand ${row.store.name}`
              : `Collapse ${row.store.name}`
        }
        title={[`${row.store.name} · ${row.server}`, row.badge?.description]
          .filter(Boolean)
          .join(' · ')}
        onClick={foldable ? onToggleCollapse : () => onOpen(row.store.id)}
      >
        <GroupMark store={row.store} size="sm" />
        <span className="t">
          <b>{row.store.name}</b>
          <small className="chat-row-identity">{row.server}</small>
          {row.status && <small>{row.status}</small>}
          {row.note && <small className="chat-row-note">{row.note}</small>}
        </span>
        {badge && (
          <span className="chat-unread" aria-label={badge.description}>
            {badge.label}
          </span>
        )}
        {foldable && (
          <span className="chat-team-chev" aria-hidden="true">
            <Icon name="chev" size={14} />
          </span>
        )}
      </button>
      {/* The channels belong to the team named above them, and say so rather
          than leaving a screen reader to infer it from the order. Collapsed,
          the group is not rendered at all, so no channel button sits in the
          tab order while it cannot be seen. */}
      {!collapsed && (
        <div
          id={channelListId}
          className="chat-channel-list"
          role="group"
          aria-label={row.store.name}
        >
          {empty && (
            <button
              type="button"
              className={open ? 'chat-channel on' : 'chat-channel'}
              aria-label={`No channels in ${row.store.name}`}
              aria-current={open ? 'page' : undefined}
              onClick={() => onOpen(row.store.id)}
            >
              <span className="t">
                <span className="n">No channels</span>
              </span>
            </button>
          )}
          {channels.map((listedChannel) => {
            const { channel, conversation } = listedChannel;
            const active = channel.id === activeChannel;
            const count = channelCount(listedChannel);
            const meta = channelMeta(listedChannel, row.entry?.blockedChannels);
            // The caption is the one thing the row has to say beside its
            // name: why it is quiet, else what was last said in it.
            const preview = conversation?.preview;
            const line = meta
              ? ''
              : previewLine(
                  conversation,
                  row.entry?.scope?.actor ?? null,
                  row.names,
                );
            const caption =
              meta ||
              line ||
              (listedChannel === lone && !preview ? 'No messages yet' : '');
            return (
              <button
                type="button"
                key={channel.id}
                className={[
                  'chat-channel',
                  active ? 'on' : '',
                  count ? 'unread' : '',
                  conversation?.hidden ? 'hidden' : '',
                  // Muted is drawn the way hidden is: the caption beside the
                  // name already says so, and the row reads at that volume.
                  conversation?.muted ? 'muted' : '',
                  row.reachable ? '' : 'off',
                ]
                  .filter(Boolean)
                  .join(' ')}
                aria-current={active ? 'page' : undefined}
                // A channel of an unreachable team opens the locked pane, which
                // states the reason. Disabling it would be the one row in the
                // column that answers nothing: the team's own row is clickable
                // and lands in the same place.
                onClick={() => onOpen(row.store.id, channel.id)}
              >
                <span className="t">
                  <span className="n">{channelTitle(channel)}</span>
                  {caption && <small>{caption}</small>}
                </span>
                {preview && !meta && row.reachable && (
                  <span className="when">
                    {previewTime(preview.insert_time, now)}
                  </span>
                )}
                {channel.admin && (
                  <span
                    className="lock"
                    title="Admins and owners only"
                    aria-label="Admins and owners only"
                  >
                    <Icon name="key" size={13} />
                  </span>
                )}
                {count && row.reachable && (
                  <span
                    className={
                      conversation?.muted ? 'chat-unread muted' : 'chat-unread'
                    }
                    aria-label={`${count} unread`}
                  >
                    {count}
                  </span>
                )}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
