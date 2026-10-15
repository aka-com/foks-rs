import { inventoryReadiness } from '../model';
import { CollectionStatus } from '../components/collection-status';
/**
 * The Chat tab's inbox column: every team with chat, at once.
 *
 * Displays the Chat tab inbox sidebar containing all teams and their channels.
 *
 * Each team header expands and collapses its channel list. The folded teams,
 * the channel order and whether rows carry previews are display preferences
 * stored through `sidebar-prefs.ts`, so the column returns as it was left.
 * When collapsed, the team header shows the aggregate unread count for its
 * channels.
 *
 * Team and channel data are supplied by `useChatInbox()`. If a team's channels
 * fail to load, an inline error with a retry button is shown in place of the
 * channel list. Teams on servers that do not support chat are listed in a
 * disabled state at the bottom. Filtering is driven by the search query passed
 * from the window header.
 */

import {
  useEffect,
  useEffectEvent,
  useId,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react';
import type { ReactNode } from 'react';
import { Icon, MenuItem, SectionLabel } from '../components';
import {
  anyDialogOpen,
  ContextMenu,
  Menu,
  Popover,
} from '/kit/overlay-primitives';
import { useOptionalToast } from '/kit/toasts';
import { normalizeCommandError } from '../bridge';
import { chatErrorText } from '../chat/error-copy';
import { useChannelCreation } from '../chat/channel-creation-provider';
import {
  chatAvailable,
  partiesOf,
  roleRank,
  plural,
  serverDisplayLabel,
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
  channelLabel,
  channelMeta,
  channelTitle,
  listChannels,
  messageDate,
  neverMessaged,
  partyNames,
  previewLine,
  previewTime,
} from '../chat/presentation';
import type { ListedChannel } from '../chat/presentation';
import { useChatInbox } from '../chat/inbox-provider';
import {
  alertChoice,
  alertsUnavailableReason,
  notificationKey,
  storedAlertMode,
  teamKey,
  teamMuted,
} from '../chat/local-contract';
import type { AlertChoice, LocalAction } from '../chat/local-contract';
import {
  ALERT_TITLES,
  MENTIONS_DETAIL,
  useLocalAlerts,
} from '../chat/notification-provider';
import type { TeamInbox } from '../chat/inbox-service';
import { useChatSendService } from '../chat/send-provider';
import { markChannelRead } from '../chat/mark-read';
import { lastPosition } from '../chat/snapshots';
import { teamUnread } from '../chat/unread';
import {
  rememberChannelPreviews,
  rememberChannelSort,
  rememberTeamFolded,
  storedChannelPreviews,
  storedChannelSort,
  storedFoldedTeams,
} from '../sidebar-prefs';
import type { ChannelSort } from '../sidebar-prefs';
import { GroupMark } from './group-mark';
import { manageReason } from './group-model';

/** The server a store belongs to, whatever its state. */
function serverFor(
  snapshot: AgentSnapshot,
  store: TeamStore,
): Server | undefined {
  return snapshot.servers.find((server) => server.profileName === store.server);
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
  return `Chat is not enabled on ${serverDisplayLabel(server)}`;
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
  /** True if channels failed to load for this team. */
  failed: boolean;
  /** Status or error message displayed in place of the channel list. */
  status: string;
  /**
   * What a synchronization that succeeded could not finish, said beside the
   * channels rather than in place of them.
   */
  note: string;
  /** `undefined` while the team's channel list has not arrived. */
  channels: ListedChannel[] | undefined;
  badge: { label: string; description: string } | null;
  /** Team subtitle showing server host and member count (if known). */
  caption: string;
  server: string;
  /** The team's people by party identifier, so a preview's sender is a name. */
  names: ReadonlyMap<string, string>;
  /** The reader's own party identifier, so their messages preview as "You". */
  actor: string | null;
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
  sort: ChannelSort,
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
  const server = teamCaption(snapshot, store, { kind: false });
  // The roster the Teams page already loads, read rather than fetched again. A
  // real team always has at least its own member, so an empty roster means it
  // has not arrived and the subtitle is the host alone rather than "0 members".
  const members = partiesOf(snapshot, store.id).length || undefined;
  return {
    store,
    entry,
    reachable,
    failed,
    status: unavailable || failure || (loading ? 'Loading channels…' : ''),
    note: !failed && reachable && entry?.data ? entry.error || entry.note : '',
    channels: entry?.data
      ? listChannels(entry.data.channels, entry.data.conversations, sort)
      : undefined,
    badge: reachable ? unread : { label: '!', description: unavailable },
    caption:
      members === undefined
        ? server
        : `${server} · ${plural(members, 'member')}`,
    server,
    names: partyNames(snapshot, store.id),
    actor: entry?.scope?.actor ?? null,
  };
}

/** A channel's own unread count, or `null` when it has none. */
function channelCount(listed: ListedChannel | undefined): string | null {
  const conversation = listed?.conversation;
  return conversation && BigInt(conversation.unread) > 0n
    ? conversation.unread
    : null;
}

/** A channel the column lists, in the order it lists them. */
interface ColumnChannel {
  team: StoreRef;
  channel: string;
  /** The row is drawn: its team is not folded, or a search overrides the fold. */
  drawn: boolean;
  /**
   * The row draws an unread count that `teamUnread` counts: the team is
   * reachable and the conversation is neither muted nor hidden.
   */
  unread: boolean;
}

/**
 * The channel Alt+Up/Down (`unread` false) or Alt+Shift+Up/Down (`unread`
 * true) moves to from the open channel, in the column's order. A plain step
 * moves to the next drawn row and stops at either end; the unread search
 * also covers the rows of folded teams, wraps around and skips the open
 * channel. With the open channel not listed, the step starts where the open
 * team's rows start (`teamStarts`), or at the column's end it moves away from.
 */
function channelStep(
  rows: readonly ColumnChannel[],
  teamStarts: ReadonlyMap<StoreRef, number>,
  team: StoreRef | undefined,
  channel: string | undefined,
  direction: 1 | -1,
  unread: boolean,
): ColumnChannel | undefined {
  const at = rows.findIndex(
    (row) => row.team === team && row.channel === channel,
  );
  const teamStart = team === undefined ? undefined : teamStarts.get(team);
  // A position between rows when the open channel is not one of them.
  const from =
    at >= 0
      ? at
      : teamStart !== undefined
        ? teamStart - 0.5
        : direction > 0
          ? -0.5
          : rows.length - 0.5;
  const indices = [...rows.keys()];
  const ahead =
    direction > 0
      ? indices.filter((index) => index > from)
      : indices.filter((index) => index < from).reverse();
  if (!unread)
    return ahead.map((index) => rows[index]).find((row) => row.drawn);
  const behind =
    direction > 0
      ? indices.filter((index) => index < from)
      : indices.filter((index) => index > from).reverse();
  return [...ahead, ...behind]
    .map((index) => rows[index])
    .find((row) => row.unread);
}

/**
 * Whether Alt+Up/Down in `target` belongs to the column rather than to the
 * field: in a multi-line field only when the caret is on its first line
 * (Up) or its last line (Down), since macOS moves the caret to a paragraph
 * boundary with Option+Up/Down, and never in a select. Lines are the text's
 * own line breaks.
 */
function caretAtEdge(target: EventTarget | null, direction: 1 | -1): boolean {
  if (!(target instanceof Element)) return true;
  // Alt+Down opens a select's list.
  if (target.matches('select')) return false;
  if (!target.matches('textarea')) return true;
  const { value, selectionStart, selectionEnd } = target;
  return direction < 0
    ? !value.slice(0, selectionStart).includes('\n')
    : !value.slice(selectionEnd).includes('\n');
}

export interface ChatTeamColumnProps {
  id?: string;
  snapshot: AgentSnapshot;
  /** The team whose conversation is mounted, when there is one. */
  selected?: StoreRef;
  /** The open channel. */
  activeChannel?: string;
  /** The clock the tab decides availability on, so the column shares it. */
  accessOptions?: AvailabilityOptions;
  onOpen: (ref: StoreRef, channel?: string) => void;
  /** Opens a channel and displays its information panel. */
  onChannelInfo: (ref: StoreRef, channel: string) => void;
  onNewChat: () => void;
  /** Opens a team's page on the Teams tab, where its setup is finished. */
  onTeams: (ref: StoreRef) => void;
  onAddPeople?: (ref: StoreRef, intent: 'add' | 'add-team' | 'invite') => void;
  /**
   * The window header's scoped search text. The column carries no field of its
   * own; it narrows to the teams and channels this matches.
   */
  query?: string;
}

export function ChatTeamColumn({
  id,
  snapshot,
  selected,
  activeChannel,
  accessOptions = {},
  onOpen,
  onChannelInfo,
  onNewChat,
  onTeams,
  onAddPeople,
  query: search = '',
}: ChatTeamColumnProps): ReactNode {
  const { service, snapshot: inbox } = useChatInbox();
  const sends = useChatSendService();
  // The channels other than the open one that hold a draft, as one string of
  // `["team","channel"]` lines. It changes when such a draft appears or goes,
  // and a string compares by value, so typing in the open channel's
  // composer, including its first and last character, does not re-render the
  // column.
  const draftKey = () => {
    const lines: string[] = [];
    for (const [team, channels] of sends.draftedChannels())
      for (const channel of channels)
        if (team !== selected || channel !== activeChannel)
          lines.push(JSON.stringify([team, channel]));
    return lines.sort().join('\n');
  };
  const drafted = new Set(
    useSyncExternalStore(sends.subscribeDrafted, draftKey, draftKey)
      .split('\n')
      .filter(Boolean),
  );
  const toasts = useOptionalToast();
  const alerts = useLocalAlerts();
  const { controller } = useChannelCreation();
  const [context, setContext] = useState<{
    x: number;
    y: number;
    team?: StoreRef;
    channel?: string;
  } | null>(null);

  // Which teams have their channel list folded away, by team id: what the
  // reader folded or unfolded since the column mounted, over what was stored
  // when it mounted. Teams arrive after the column does, so the stored set is
  // consulted per team rather than copied into state once.
  const [storedFold] = useState(storedFoldedTeams);
  const [folds, setFolds] = useState<ReadonlyMap<string, boolean>>(
    () => new Map(),
  );
  const collapsedTeam = (id: string) => folds.get(id) ?? storedFold(id);
  const toggleCollapsed = (id: string) => {
    const folded = !collapsedTeam(id);
    setFolds((prior) => new Map(prior).set(id, folded));
    rememberTeamFolded(id, folded);
  };
  const [sort, setSort] = useState(storedChannelSort);
  const [previews, setPreviews] = useState(storedChannelPreviews);
  const chooseSort = (next: ChannelSort) => {
    setSort(next);
    rememberChannelSort(next);
  };
  const showPreviews = (shown: boolean) => {
    setPreviews(shown);
    rememberChannelPreviews(shown);
  };
  // The column searches names, not messages. Message text is found in the
  // open conversation's find bar, over the rows it holds and the older pages
  // it scans; the agent has no message index to search the column with.
  const filter = search;
  const query = filter.trim().toLowerCase();
  // The clock is the tab's, but the object it arrives in is rebuilt every
  // render, so the cache is keyed on the moment rather than on its identity —
  // and on the second rather than on the fraction of a millisecond the tab
  // reads, which no two renders share and which would make the cache a cost
  // with no hit. Availability changes on the second; a check-in that lapses
  // between two ticks is seen on the next one.
  const { nowSeconds, agentReady, catalogReady } = accessOptions;
  const second = nowSeconds === undefined ? undefined : Math.floor(nowSeconds);
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
        teamRow(snapshot, inbox, store, options, store.id === selected, sort),
      ),
    };
  }, [snapshot, inbox, second, agentReady, catalogReady, selected, sort]);
  // Preview times are stated against the tab's clock when it supplies one,
  // the clock the rest of the column reads.
  const now = second === undefined ? Date.now() : second * 1000;
  const dimmed = withoutChat.filter(
    (store) => !query || store.name.toLowerCase().includes(query),
  );
  let listedChannels = 0;
  const allChannels = rows.reduce(
    (total, row) => total + (row.channels ?? []).length,
    0,
  );
  const contextTeam = context?.team
    ? snapshot.stores.find(
        (store) => store.id === context.team && store.kind === 'team',
      )
    : undefined;
  const contextRow = rows.find((row) => row.store.id === context?.team);
  const contextChannel = contextRow?.channels?.find(
    ({ channel }) => channel.id === context?.channel,
  );
  const openReason = !contextChannel
    ? 'This channel is no longer available.'
    : !contextRow?.reachable
      ? contextRow?.status || 'Chat is unavailable for this team.'
      : !contextChannel.channel.readable ||
          contextRow.entry?.blockedChannels.has(contextChannel.channel.id)
        ? 'You do not have access to this channel.'
        : undefined;
  // Mark as read is offered while the channel's row draws an unread count —
  // a channel of an unreachable team draws none — and moves the read pointer
  // to the newest position the inbox states, the one the count is counted up
  // to.
  const contextUnread =
    contextRow?.reachable === true && channelCount(contextChannel);
  const markContextRead = () => {
    const conversation = contextChannel?.conversation;
    const team = contextRow?.store.id;
    if (!conversation || !team) return;
    const title = channelTitle(conversation.channel);
    markChannelRead(
      (action) => sends.request(team, action),
      service,
      team,
      conversation.channel.id,
      String(lastPosition(conversation)),
    ).catch((cause: unknown) => {
      const error = normalizeCommandError(cause);
      if (error.code === 'cancelled') return;
      toasts?.show(`${title} was not marked as read. ${chatErrorText(error)}`, {
        tone: 'warning',
      });
    });
  };
  // Mute and the team's alert default are device-local settings, keyed by
  // the scope the team's inbox was synchronized under.
  const contextScope = contextRow?.entry?.scope;
  const localSettings = alerts?.session?.settings;
  const localReason = !contextScope
    ? 'Chat is unavailable for this team.'
    : !localSettings
      ? 'Local settings are loading.'
      : undefined;
  const contextMuted =
    !!contextScope &&
    !!context?.channel &&
    !!localSettings?.muted.includes(
      notificationKey(contextScope, context.channel),
    );
  // A team's mute covers its channels, so a channel's own mute cannot be
  // changed until the team is unmuted.
  const contextTeamMuted =
    !!contextScope && !!localSettings && teamMuted(localSettings, contextScope);
  const teamMuteReason =
    contextTeamMuted && contextRow
      ? `All ${contextRow.store.name} channels are muted from the team menu.`
      : undefined;
  const teamMode = contextScope
    ? localSettings?.teams[teamKey(contextScope)]
    : undefined;
  const contextTeamMode: AlertChoice | 'inherit' =
    teamMode === undefined ? 'inherit' : alertChoice(teamMode);
  const configureLocal = (
    change: Pick<
      Extract<LocalAction, { action: 'configure' }>,
      'channel' | 'muted' | 'mode'
    >,
    failed: string,
  ) => {
    const team = contextRow?.store.id;
    if (!alerts || !team || !contextScope) return;
    void alerts
      .configure({
        action: 'configure',
        storeId: team,
        scope: contextScope,
        ...change,
      })
      .then((error) => {
        if (error) toasts?.show(`${failed} ${error}`, { tone: 'warning' });
      });
  };
  const createReason = !controller
    ? 'Channel creation is unavailable in this window.'
    : !rows.some(
          (row) =>
            row.reachable &&
            partiesOf(snapshot, row.store.id).some(
              (party) =>
                party.label === 'you' && roleRank(party.destination_role) >= 1,
            ),
        )
      ? 'No team with channel creation access is currently available.'
      : undefined;
  const closeContext = () => setContext(null);
  // The row the menu was opened from: the focused element for the keyboard,
  // the channel row or team heading under the pointer. Dismissing the menu
  // returns focus there rather than to the document body.
  const contextAnchor = useRef<HTMLElement | null>(null);
  const dismissContext = () => {
    setContext(null);
    const anchor = contextAnchor.current;
    if (anchor?.isConnected && !anyDialogOpen())
      anchor.focus({ preventScroll: true });
  };
  // Opens the menu for what `target` sits in: a channel row, a team, or the
  // column itself. A target inside an open menu opens nothing.
  const showContext = (
    target: EventTarget,
    x: number,
    y: number,
    keyboard: boolean,
  ): boolean => {
    if (!(target instanceof Element) || target.closest('[role="menu"]'))
      return false;
    const channel = target.closest<HTMLElement>('[data-chat-channel]');
    const team = target.closest<HTMLElement>('[data-chat-team]');
    if (keyboard && !team) return false;
    contextAnchor.current =
      keyboard && target instanceof HTMLElement
        ? target
        : (channel ??
          team?.querySelector<HTMLElement>('button.chat-team-head') ??
          null);
    setContext({
      x,
      y,
      team: team?.dataset.chatTeam,
      channel: channel?.dataset.chatChannel,
    });
    return true;
  };
  let listed = 0;
  const teamRows: ReactNode[] = [];
  // Every listed channel, in order, for Alt+Up/Down, with where each team's
  // channels start, including a team the search leaves out or that has none.
  const columnChannels: ColumnChannel[] = [];
  const teamStarts = new Map<StoreRef, number>();
  rows.forEach((row) => {
    teamStarts.set(row.store.id, columnChannels.length);
    const named = row.store.name.toLowerCase().includes(query);
    const matching = (row.channels ?? []).filter(
      ({ channel }) =>
        !query || named || channelTitle(channel).toLowerCase().includes(query),
    );
    if (query && !named && !matching.length) return;
    listed += 1;
    listedChannels += matching.length;
    const drawn = Boolean(query) || !collapsedTeam(row.store.id);
    for (const listedChannel of matching) {
      const { conversation } = listedChannel;
      columnChannels.push({
        team: row.store.id,
        channel: listedChannel.channel.id,
        drawn,
        unread:
          row.reachable &&
          channelCount(listedChannel) !== null &&
          !conversation?.muted &&
          !conversation?.hidden,
      });
    }
    teamRows.push(
      <TeamHeading
        key={row.store.id}
        row={row}
        channels={matching}
        selected={selected}
        activeChannel={row.store.id === selected ? activeChannel : undefined}
        drafts={drafted}
        // A search in progress overrides a fold: a channel the query
        // matched inside a collapsed team has to be seen to explain why its
        // team matched at all. The fold itself is untouched underneath, and
        // returns as soon as the query is cleared.
        collapsed={query ? false : collapsedTeam(row.store.id)}
        previews={previews}
        now={now}
        onToggleCollapse={() => toggleCollapsed(row.store.id)}
        onOpen={onOpen}
        // Retrying one team reloads that team alone: every other team's
        // channels, and the conversation open beside the column, are untouched.
        onRetry={() => service.invalidate(row.store.id)}
      />,
    );
  });
  // Alt+Up/Down opens the previous or next channel row, and Alt+Shift+Up/Down
  // the previous or next one with unread messages, unfolding its team when
  // the team is folded. The listener is on the document, beyond a dialog's
  // event boundary, so it does nothing while a dialog is open, as the rail's
  // Ctrl+Tab does.
  const step = useEffectEvent((event: KeyboardEvent): void => {
    if (
      (event.key !== 'ArrowUp' && event.key !== 'ArrowDown') ||
      !event.altKey ||
      event.ctrlKey ||
      event.metaKey ||
      event.defaultPrevented ||
      anyDialogOpen()
    )
      return;
    const direction = event.key === 'ArrowUp' ? -1 : 1;
    if (!caretAtEdge(event.target, direction)) return;
    event.preventDefault();
    const next = channelStep(
      columnChannels,
      teamStarts,
      selected,
      activeChannel,
      direction,
      event.shiftKey,
    );
    if (next) {
      // The unfold lasts while the column is mounted; the stored fold is left
      // as the reader set it.
      if (!next.drawn)
        setFolds((prior) => new Map(prior).set(next.team, false));
      onOpen(next.team, next.channel);
    } else if (event.shiftKey)
      toasts?.show('No other channel has unread messages.');
  });
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => step(event);
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, []);
  return (
    <aside
      id={id}
      className="chat-inbox"
      aria-label="Chat inbox"
      onContextMenu={(event) => {
        if (showContext(event.target, event.clientX, event.clientY, false))
          event.preventDefault();
      }}
      // The keyboard opens the same menu from a focused channel row or team
      // heading, with the ContextMenu key or Shift+F10, at the row's corner.
      onKeyDown={(event) => {
        if (
          event.key !== 'ContextMenu' &&
          !(event.key === 'F10' && event.shiftKey)
        )
          return;
        if (!(event.target instanceof HTMLElement)) return;
        const bounds = event.target.getBoundingClientRect();
        if (!showContext(event.target, bounds.left, bounds.bottom, true))
          return;
        event.preventDefault();
        event.stopPropagation();
      }}
    >
      {/* Screen reader status announcement for search filtering results.
          The element is rendered persistently to ensure aria-live announcements
          fire. */}
      <p className="offscreen" role="status">
        {query
          ? `${listed + dimmed.length} of ${teams.length + withoutChat.length} teams and ${listedChannels} of ${allChannels} channels match ${filter}.`
          : ''}
      </p>
      <div className="chat-inbox-scroll">
        {/* Nothing to list and nothing to search: the column says what would
            be here, and the pane beside it says how to get one. */}
        {!teams.length &&
        !withoutChat.length &&
        inventoryReadiness(snapshot, 'teams') !== 'ready' ? (
          <CollectionStatus
            state={inventoryReadiness(snapshot, 'teams')}
            label="teams"
          />
        ) : !teams.length && !withoutChat.length ? (
          <div className="chat-inbox-none">
            <b>No teams yet</b>
            <span>Teams appear here with their channels.</span>
          </div>
        ) : null}
        {/* The heading draws only once there is a team under it: a search
            that clears every team drops the heading with it. The "+" it
            carries is the column's one action, and it opens the same channel
            creation the conversation pane does. */}
        {teamRows.length > 0 && (
          <SectionLabel
            className="chat-inbox-label"
            action={
              <>
                <ColumnDisplayMenu
                  sort={sort}
                  previews={previews}
                  onSort={chooseSort}
                  onPreviews={showPreviews}
                />
                <button
                  type="button"
                  className="chat-inbox-add"
                  aria-label="New channel"
                  title="New channel"
                  onClick={onNewChat}
                >
                  <Icon name="plus" size={14} />
                </button>
              </>
            }
          >
            Teams
          </SectionLabel>
        )}
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
                  data-chat-team={store.id}
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
      {context ? (
        <ContextMenu
          point={context}
          className="menu-portal"
          onClose={dismissContext}
        >
          <Menu
            className="menu"
            aria-label="Chat sidebar actions"
            initialFocus="first"
            onClose={dismissContext}
          >
            {context.channel ? (
              <>
                <MenuItem
                  icon="chat"
                  reason={openReason}
                  onClick={() => {
                    closeContext();
                    onOpen(context.team!, context.channel);
                  }}
                >
                  Open channel
                </MenuItem>
                {/* Renaming and deleting channels are not offered until
                    channel management ships; an item that can never apply
                    is not listed. */}
                {contextUnread && (
                  <MenuItem
                    icon="circleCheck"
                    reason={openReason}
                    // Marking leaves the reader in the column, so focus
                    // returns to the row as it does on dismissal.
                    onClick={() => {
                      dismissContext();
                      markContextRead();
                    }}
                  >
                    Mark as read
                  </MenuItem>
                )}
                {alerts && (
                  <MenuItem
                    icon={contextMuted ? 'bell' : 'bellOff'}
                    title="Applies to this device only"
                    reason={
                      !contextChannel
                        ? 'This channel is no longer available.'
                        : (localReason ?? teamMuteReason)
                    }
                    // Like Mark as read, this leaves the reader in the column.
                    onClick={() => {
                      dismissContext();
                      const title = contextChannel
                        ? channelTitle(contextChannel.channel)
                        : 'The channel';
                      configureLocal(
                        { channel: context.channel, muted: !contextMuted },
                        contextMuted
                          ? `${title} was not unmuted.`
                          : `${title} was not muted.`,
                      );
                    }}
                  >
                    {contextMuted ? 'Unmute channel' : 'Mute channel'}
                  </MenuItem>
                )}
                <MenuItem
                  icon="info"
                  reason={
                    contextChannel
                      ? undefined
                      : 'This channel is no longer available.'
                  }
                  onClick={() => {
                    closeContext();
                    onChannelInfo(context.team!, context.channel!);
                  }}
                >
                  Channel info
                </MenuItem>
              </>
            ) : context.team ? (
              <>
                <MenuItem
                  icon="users"
                  reason={
                    contextTeam
                      ? undefined
                      : 'This team is no longer available.'
                  }
                  onClick={() => {
                    closeContext();
                    onTeams(context.team!);
                  }}
                >
                  Go to team
                </MenuItem>
                {alerts && contextRow && (
                  <TeamAlertItems
                    team={contextRow.store.name}
                    value={contextTeamMode}
                    reason={
                      localReason ??
                      (localSettings &&
                      alerts.session &&
                      !alerts.session.available
                        ? alertsUnavailableReason(alerts.session)
                        : undefined)
                    }
                    muted={contextTeamMuted}
                    muteReason={localReason}
                    onMute={(muted) => {
                      dismissContext();
                      configureLocal(
                        { muted },
                        muted
                          ? `${contextRow.store.name} channels were not muted.`
                          : `${contextRow.store.name} channels were not unmuted.`,
                      );
                    }}
                    onChoose={(choice) => {
                      dismissContext();
                      configureLocal(
                        {
                          mode:
                            choice === 'inherit'
                              ? null
                              : storedAlertMode(choice),
                        },
                        `Alerts for ${contextRow.store.name} were not changed.`,
                      );
                    }}
                  />
                )}
              </>
            ) : (
              <MenuItem
                icon="plus"
                reason={createReason}
                onClick={() => {
                  closeContext();
                  onNewChat();
                }}
              >
                New channel
              </MenuItem>
            )}
            {context.team && onAddPeople ? (
              <>
                <div className="menu-separator" role="separator" />
                {(
                  [
                    ['add', 'user', 'Add FOKS user…', 'roster'],
                    ['add-team', 'users', 'Add FOKS team…', 'federation'],
                    ['invite', 'door', 'Invite new user…', 'roster'],
                  ] as const
                ).map(([intent, icon, label, source]) => (
                  <MenuItem
                    key={intent}
                    icon={icon}
                    reason={
                      contextTeam?.kind === 'team'
                        ? manageReason(snapshot, contextTeam, source)
                        : 'This team is no longer available.'
                    }
                    onClick={() => {
                      closeContext();
                      onAddPeople(context.team!, intent);
                    }}
                  >
                    {label}
                  </MenuItem>
                ))}
              </>
            ) : null}
          </Menu>
        </ContextMenu>
      ) : null}
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

/**
 * A team menu's alert default: the mode its channels without their own take
 * on this device, as radio items, so each choice is one arrow key away; and
 * the team's mute, which takes every channel of the team, including those
 * created later, out of the unread totals on this device.
 */
function TeamAlertItems({
  team,
  value,
  reason,
  muted,
  muteReason,
  onMute,
  onChoose,
}: {
  team: string;
  value: AlertChoice | 'inherit';
  /** Why the choices do not apply, when they do not. */
  reason?: string;
  /** The team is muted on this device. */
  muted: boolean;
  /** Why the mute cannot be changed, when it cannot. */
  muteReason?: string;
  onMute: (muted: boolean) => void;
  onChoose: (choice: AlertChoice | 'inherit') => void;
}): ReactNode {
  const descriptionId = useId();
  const muteDescriptionId = useId();
  const choices = [
    ['inherit', 'Device setting'],
    ['all', ALERT_TITLES.all],
    ['mentions', ALERT_TITLES.mentions],
    ['none', ALERT_TITLES.none],
  ] as const;
  return (
    <>
      <div className="menu-separator" role="separator" />
      <div className="cap" role="presentation">
        Alerts for {team}
      </div>
      {choices.map(([choice, title]) => (
        <button
          type="button"
          key={choice}
          className="chat-team-alert"
          role="menuitemradio"
          aria-checked={value === choice}
          aria-disabled={reason ? true : undefined}
          aria-describedby={reason ? descriptionId : undefined}
          title={
            reason ??
            (choice === 'mentions'
              ? MENTIONS_DETAIL
              : 'For channels without their own setting, on this device')
          }
          onClick={reason ? undefined : () => onChoose(choice)}
        >
          <Icon name="check" />
          {title}
        </button>
      ))}
      {reason ? (
        <span id={descriptionId} className="offscreen">
          {reason}
        </span>
      ) : null}
      <button
        type="button"
        className="chat-team-alert"
        role="menuitemcheckbox"
        aria-checked={muted}
        aria-disabled={muteReason ? true : undefined}
        aria-describedby={muteReason ? muteDescriptionId : undefined}
        title={muteReason ?? 'Applies to this device only'}
        onClick={muteReason ? undefined : () => onMute(!muted)}
      >
        <Icon name="check" />
        Mute all channels
      </button>
      {muteReason ? (
        <span id={muteDescriptionId} className="offscreen">
          {muteReason}
        </span>
      ) : null}
    </>
  );
}

/**
 * The column's sort and display control, beside its "+": the trigger names
 * the current order and opens a menu of the two orders and the previews
 * switch. Choosing keeps the menu open, so the orders can be compared; Escape
 * or a click outside closes it and returns focus to the trigger.
 */
function ColumnDisplayMenu({
  sort,
  previews,
  onSort,
  onPreviews,
}: {
  sort: ChannelSort;
  previews: boolean;
  onSort: (sort: ChannelSort) => void;
  onPreviews: (shown: boolean) => void;
}): ReactNode {
  const anchorRef = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const close = () => setOpen(false);
  const choices = [
    ['alphabetical', 'Alphabetical', '#general first, then A to Z'],
    ['recent', 'Recent activity', 'Newest message first'],
  ] as const;
  return (
    <>
      <button
        type="button"
        ref={anchorRef}
        className="chat-inbox-sort"
        aria-haspopup="menu"
        aria-expanded={open}
        // The name contains what the trigger shows, so a voice command
        // that speaks the visible label finds it.
        aria-label={`Sort and display, ${sort === 'recent' ? 'Recent activity' : 'A–Z (alphabetical)'}`}
        title="Sort and display"
        onClick={() => setOpen((was) => !was)}
        onKeyDown={(event) => {
          if (event.key !== 'ArrowDown' || open) return;
          event.preventDefault();
          setOpen(true);
        }}
      >
        <span>{sort === 'recent' ? 'Recent' : 'A–Z'}</span>
        <Icon name="chevronDown" size={13} />
      </button>
      {open && (
        <Popover
          anchorRef={anchorRef}
          className="menu-portal"
          align="end"
          gap={4}
          onClose={close}
        >
          <Menu
            className="menu chat-sort-menu"
            anchorRef={anchorRef}
            onClose={close}
            aria-label="Sort and display"
            initialFocus="first"
          >
            <div className="cap" role="presentation">
              Sort channels
            </div>
            {choices.map(([value, title, detail]) => (
              <button
                type="button"
                key={value}
                role="menuitemradio"
                aria-checked={sort === value}
                onClick={() => onSort(value)}
              >
                <Icon name="check" />
                <span className="menu-choice">
                  {title}
                  <small>{detail}</small>
                </span>
              </button>
            ))}
            <div className="menu-separator" role="separator" />
            <button
              type="button"
              role="menuitemcheckbox"
              aria-checked={previews}
              onClick={() => onPreviews(!previews)}
            >
              <Icon name="check" />
              <span className="menu-choice">
                Show previews
                <small>Last message and time under each channel</small>
              </span>
            </button>
          </Menu>
        </Popover>
      )}
    </>
  );
}

/** A team: a heading that folds its channels, and the channels under it. */
function TeamHeading({
  row,
  channels,
  selected,
  activeChannel,
  drafts,
  collapsed,
  previews,
  now,
  onToggleCollapse,
  onOpen,
  onRetry,
}: {
  row: TeamRow;
  channels: readonly ListedChannel[];
  /** The team whose conversation is mounted beside the column. */
  selected?: StoreRef;
  activeChannel?: string;
  /** `["team","channel"]` for each channel but the open one with a draft. */
  drafts: ReadonlySet<string>;
  /** The channel list is folded away; stored by the column, keyed by team id. */
  collapsed: boolean;
  /** Each channel row carries its last message and that message's time. */
  previews: boolean;
  /** The clock preview times are stated against, in milliseconds. */
  now: number;
  onToggleCollapse: () => void;
  onOpen: (ref: StoreRef, channel?: string) => void;
  /** Reloads the channel list for this team. */
  onRetry: () => void;
}): ReactNode {
  const [retrying, setRetrying] = useState(false);
  const retryFeedback = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  useEffect(
    () => () => {
      if (retryFeedback.current !== undefined)
        clearTimeout(retryFeedback.current);
    },
    [],
  );
  // A heading's own count is normally the sum of the counts already drawn
  // beside its channels, so only what the channels cannot say is drawn here:
  // that the team cannot be reached, that its count is degraded, that it is
  // going stale. Collapsed, the channels are not drawn at all, so the heading
  // carries their total instead — nothing is lost by folding the list away.
  // The channel list this team's row expands, named so the row can point at
  // it rather than leaving the relationship to visual order alone.
  const channelListId = useId();
  const stopped = row.entry?.state === 'blocked';
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
  // A known empty list folds its placeholder row just like a channel list, and
  // so does the row that stands in for a channel list that could not be read.
  // A team whose list has not arrived opens its own pane, which states
  // the reason: loading, locked, or unavailable.
  const foldable = channels.length > 0 || empty || row.failed;
  const open = activeChannel !== undefined || row.store.id === selected;
  return (
    <div
      className={collapsed ? 'chat-team collapsed' : 'chat-team'}
      data-chat-team={row.store.id}
    >
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
        onClick={() => {
          if (!foldable) {
            onOpen(row.store.id);
            return;
          }
          // A team with one channel opens it as well. The click is still the
          // fold control its name states, so the fold toggles and is stored
          // as any other.
          onToggleCollapse();
          if (row.channels?.length === 1) {
            onOpen(row.store.id, row.channels[0].channel.id);
          }
        }}
      >
        <GroupMark store={row.store} size="sm" />
        <span className="t">
          <b>{row.store.name}</b>
          {!(row.status && !row.failed) && !row.note && (
            <small className="chat-row-identity">{row.caption}</small>
          )}
          {/* A channel list that could not be read is stated by the row that
              stands in for the channels, not twice. */}
          {row.status && !row.failed && <small>{row.status}</small>}
          {row.note && <small className="chat-row-note">{row.note}</small>}
        </span>
        {/* A team whose channels are unknown has no count to give: the glyph
            says the count is withheld rather than zero. */}
        {row.failed ? (
          <span
            className="chat-team-warn"
            title={
              stopped ? 'Channels stopped' : 'Channels could not be loaded'
            }
            aria-label={
              stopped ? 'Channels stopped' : 'Channels could not be loaded'
            }
          >
            <Icon name="alert" size={14} />
          </span>
        ) : (
          badge && (
            <span className="chat-unread" aria-label={badge.description}>
              {badge.label}
            </span>
          )
        )}
        {foldable && (
          <span className="chat-team-chev" aria-hidden="true">
            <Icon name="chevronDown" size={14} />
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
          {/* The failure is drawn where the missing channels would have been,
              so the team stays recognizable and the retry is beside what it
              reloads. */}
          {row.failed && (
            <div
              className="chat-channel fail"
              role="status"
              title={
                stopped
                  ? `${row.status} · Lock and unlock FOKS to start a new chat session.`
                  : row.status
              }
            >
              {/* The same sentence the row's warning glyph is labelled
                  with, so the two do not name the failure differently. */}
              <span className="n">
                {stopped ? 'Channels stopped' : 'Channels could not be loaded'}
              </span>
              {/* A stopped team is not retried from here: its synchronization
                  is held until a new chat session starts, so the row states
                  the condition rather than offering a button that would
                  reload nothing. The open team's pane states the failure and
                  carries the retry, so the row does not offer a second one. */}
              {!stopped && row.store.id !== selected && (
                <button
                  type="button"
                  className="chat-channel-retry"
                  disabled={retrying}
                  aria-live="polite"
                  onClick={() => {
                    setRetrying(true);
                    onRetry();
                    if (retryFeedback.current !== undefined)
                      clearTimeout(retryFeedback.current);
                    retryFeedback.current = setTimeout(
                      () => setRetrying(false),
                      1_000,
                    );
                  }}
                >
                  {retrying ? 'Retrying…' : 'Retry'}
                </button>
              )}
            </div>
          )}
          {empty && (
            <button
              type="button"
              className={
                open
                  ? 'chat-channel no-channels on'
                  : 'chat-channel no-channels'
              }
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
            const stopped = row.entry?.blockedChannels.has(channel.id) ?? false;
            // A channel row is its name and why it is quiet — restricted,
            // hidden, muted, stopped. With previews on, a second line carries
            // the last message and the first line its time; a channel the
            // inbox states nothing has been said in says so there. A missing
            // preview alone is not taken to mean that, since a stopped
            // channel's preview is withheld and a synchronization can leave
            // one unavailable. With previews off, only the lone general
            // channel of a team that has never been used says so, beside
            // its name.
            const preview = conversation?.preview;
            const silent =
              channel.readable && !stopped && neverMessaged(listedChannel);
            const line =
              previews && channel.readable && !stopped
                ? previewLine(conversation, row.actor, row.names) ||
                  (silent ? 'No messages yet' : '')
                : '';
            const when =
              previews && preview && channel.readable && !stopped
                ? previewTime(preview.insert_time, now)
                : '';
            const caption =
              meta ||
              (!previews && listedChannel === lone && silent
                ? 'No messages yet'
                : '');
            // The open channel's draft is in the composer beside the column,
            // so only other channels are marked.
            const draft = drafts.has(
              JSON.stringify([row.store.id, channel.id]),
            );
            const name = [channelTitle(channel), caption, draft ? 'Draft' : '']
              .filter(Boolean)
              .join(' · ');
            const lock = channel.admin && (
              <span
                className="lock"
                title="Admins and owners only"
                aria-label="Admins and owners only"
              >
                <Icon name="key" size={13} />
              </span>
            );
            const badge = count && row.reachable && (
              <span
                className={
                  conversation?.muted ? 'chat-unread muted' : 'chat-unread'
                }
                aria-label={`${count} unread`}
              >
                {count}
              </span>
            );
            const label = (
              <span className="t">
                <span className="n">{channelLabel(channel)}</span>
                {caption && <small> · {caption}</small>}
              </span>
            );
            const draftMark = draft && (
              <span className="chat-channel-draft" aria-hidden="true">
                <Icon name="pencil" size={12} />
                Draft
              </span>
            );
            return (
              <button
                type="button"
                key={channel.id}
                data-chat-channel={channel.id}
                className={[
                  'chat-channel',
                  previews ? 'two' : '',
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
                // The "#" is drawn as its own faint glyph, so the row states
                // its own name rather than leaving a reader to assemble
                // "#" and "general" out of two elements. The preview and its
                // time follow the name, as they are drawn.
                aria-label={[name, line, when].filter(Boolean).join(', ')}
                // A channel of an unreachable team opens the locked pane, which
                // states the reason. Disabling it would be the one row in the
                // column that answers nothing: the team's own row is clickable
                // and lands in the same place.
                onClick={() => onOpen(row.store.id, channel.id)}
              >
                <span className="hash" aria-hidden="true">
                  #
                </span>
                {previews ? (
                  <span className="chat-channel-body">
                    <span className="chat-channel-head">
                      {label}
                      {lock}
                      {draftMark}
                      {when && (
                        <time
                          dateTime={messageDate(
                            preview!.insert_time,
                          )?.toISOString()}
                        >
                          {when}
                        </time>
                      )}
                    </span>
                    <span className="chat-channel-foot">
                      <span className="chat-channel-preview">{line}</span>
                      {badge}
                    </span>
                  </span>
                ) : (
                  <>
                    {label}
                    {lock}
                    {draftMark}
                    {badge}
                  </>
                )}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
