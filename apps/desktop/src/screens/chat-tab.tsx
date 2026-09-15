/**
 * The Chat tab: the inbox column beside one conversation.
 *
 * The column belongs to the tab, not to a conversation: it is built from the
 * store list and the per-team inbox projections the rail already reads, so it
 * lists every team at once and survives a team switch while exactly one
 * conversation — the one the location names — is mounted and unmounted beside
 * it. `{kind:'chat'}` with no `ref` opens the conversation with the most
 * recent message and says so, falls back to the first team that has chat, and
 * says how chat gets turned on when no team has any.
 */

import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { Button, Icon } from '../components';
import { chatAvailable, storeOf } from '../model';
import type {
  AgentSnapshot,
  AvailabilityOptions,
  StoreRef,
  TeamStore,
} from '../model';
import type { Bridge } from '../bridge';
import type { Location, NavigateOptions } from '../location';
import { rememberChatLocation, rememberedChatRef } from '../location';
import { useSidebarInbox } from '../chat/inbox-provider';
import { channelTitle, listChannels, openChannel } from '../chat/presentation';
import { ChatScreen } from './chat-screen';
import { ChannelInfoPanel } from './chat-info';
import { NewChatSheet } from './chat-new';
import {
  ChatTeamColumn,
  chatTeams,
  noChatReason,
  openingConversation,
} from './chat-teams';
import './chat.css';

export interface ChatTabProps {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  location: Extract<Location, { kind: 'chat' }>;
  /**
   * The options are carried for the tab's own resolution of `{kind:'chat'}`,
   * which is forced: everything the reader asks for goes through the guards.
   */
  onNavigate: (location: Location, options?: NavigateOptions) => void;
  accessNow?: () => number;
  /**
   * The shell's access generation per server. The tab resolves which team it
   * opens, so it is the only place that knows which server's generation guards
   * the conversation and the New chat sheet.
   */
  accessGenerations?: ReadonlyMap<string, number>;
}

const NO_GENERATIONS: ReadonlyMap<string, number> = new Map();

const systemAccessNow = () => Date.now() / 1000;

/** The conversation the tab opened on its own, and why it chose that one. */
interface AutoOpened {
  ref: StoreRef;
  channel?: string;
  /** Chosen for its last message rather than for being the first team. */
  recent: boolean;
}

export function ChatTab({
  snapshot,
  bridge,
  location,
  onNavigate,
  accessNow = systemAccessNow,
  accessGenerations = NO_GENERATIONS,
}: ChatTabProps): ReactNode {
  const inbox = useSidebarInbox();
  // One clock for the whole tab: the column, the pane and the fallback all
  // read availability off the moment this render started.
  const accessOptions: AvailabilityOptions = { nowSeconds: accessNow() };
  const teams = chatTeams(snapshot);
  // A team the location names but that has no chat is not opened: the pane
  // says why, and the column stays as it is.
  const named = location.ref
    ? teams.find((store) => store.id === location.ref)
    : undefined;
  const [autoOpened, setAutoOpened] = useState<AutoOpened | null>(null);
  const [noteDismissed, setNoteDismissed] = useState(false);
  // The location the tab navigated to on its own, so a location that arrived
  // from anywhere else is recognized as not the tab's.
  const wrote = useRef<{ ref: StoreRef; channel?: string } | null>(null);
  // Initial selection is provisional while inboxes respond, allowing a more
  // recent conversation to replace it. Selection becomes final when the chosen
  // team's inbox loads. A location set externally, such as by a notification,
  // column selection, or restored rail state, also finalizes selection so later
  // inbox updates do not change the requested team. A location without a team
  // starts initial selection.
  const settled = useRef(false);
  if (!location.ref) settled.current = false;
  else if (
    wrote.current?.ref !== location.ref ||
    wrote.current.channel !== location.channel ||
    inbox.get(location.ref)?.data !== undefined
  )
    settled.current = true;
  const resolving =
    !settled.current &&
    (!location.ref || (autoOpened !== null && !noteDismissed));
  const opening = resolving
    ? openingConversation(snapshot, inbox, accessOptions)
    : undefined;
  const openingRef = opening === 'pending' ? undefined : opening?.ref;
  const openingChannel = opening === 'pending' ? undefined : opening?.channel;
  const open: TeamStore | undefined = location.ref
    ? named
    : teams.find((store) => store.id === openingRef);
  const ref: StoreRef | undefined = open?.id;
  const [info, setInfo] = useState(false);
  const [newChat, setNewChat] = useState<{ team?: StoreRef } | null>(null);
  // The ⓘ toggle takes focus back when the panel it opened closes, and the
  // conversation takes it when the note that was focused is dismissed.
  const infoToggle = useRef<HTMLButtonElement | null>(null);
  const conversation = useRef<HTMLElement | null>(null);
  const search = useRef<HTMLInputElement | null>(null);
  // A New chat sheet whose submission is unresolved cannot be dismissed, so a
  // team switch must not take it away either.
  const newChatUnresolved = useRef(false);
  // The tab with no conversation chosen is not a place to stay: it resolves to
  // the conversation with the most recent message, else to the first team that
  // has chat, and the location remembers the choice.
  useEffect(() => {
    if (!openingRef) return;
    if (location.ref === openingRef && location.channel === openingChannel)
      return;
    setAutoOpened({
      ref: openingRef,
      channel: openingChannel,
      recent: Boolean(openingChannel),
    });
    wrote.current = { ref: openingRef, channel: openingChannel };
    onNavigate(
      openingChannel
        ? { kind: 'chat', ref: openingRef, channel: openingChannel }
        : { kind: 'chat', ref: openingRef },
      // Resolving `{kind:'chat'}` into the team and channel it stands for is
      // the tab canonicalizing the address of the page already open, not a
      // move the reader asked for, so no screen is asked about it. A guard
      // that prompted here would put a question between the reader and a Chat
      // tab that has not finished opening.
      { force: true },
    );
  }, [location.ref, location.channel, openingRef, openingChannel, onNavigate]);
  // A half-finished New chat belongs to the team it was opened in: a switch —
  // a notification activation, say — closes it rather than rebinding it to the
  // team that arrives. A submission the agent has already been given is the
  // exception: it has to be settled where it was made.
  useEffect(() => {
    if (!newChatUnresolved.current) setNewChat(null);
  }, [ref]);
  // The rail's Chat tab returns to the team and channel that were open. A
  // location naming a team without chat does not erase that memory: only the
  // remembered team losing chat forgets it. The teams are a newline-joined
  // key, because the effect depends on which teams have chat, not on the
  // array a render happened to build.
  const chatRefs = teams.map((store) => store.id).join('\n');
  useEffect(() => {
    if (ref) {
      rememberChatLocation({ ...location, ref });
      return;
    }
    const remembered = rememberedChatRef();
    if (remembered && !chatRefs.split('\n').includes(remembered))
      rememberChatLocation(null);
  }, [chatRefs, location, ref]);
  const entry = ref ? inbox.get(ref) : undefined;
  const listed = entry?.data
    ? listChannels(entry.data.channels, entry.data.conversations)
    : [];
  // The channel the pane will mount, named before the navigation that carries
  // it has committed: for the one render between resolving a conversation and
  // the location arriving, the column would otherwise mark the team's first
  // channel current while the pane mounts the one that was chosen. It is the
  // resolver's channel only while the team is the resolver's team as well: a
  // channel of the team it was about to open is not a channel of the team the
  // location names.
  const wanted =
    location.channel ?? (openingRef === ref ? openingChannel : undefined);
  const channel = openChannel(listed, wanted);
  const loading =
    Boolean(open) && (!entry || (entry.state === 'loading' && !entry.error));
  useEffect(() => {
    if (info && open && !loading && !channel) setInfo(false);
  }, [channel, info, loading, open]);
  // The panel stays through a team switch: its channel arrives with the new
  // team's inbox, and the panel says so meanwhile rather than flickering out.
  const showInfo = info && Boolean(open);
  const autoNote =
    open && autoOpened?.ref === open.id && !noteDismissed ? autoOpened : null;
  const autoChannel = autoNote?.channel
    ? listed.find(({ channel }) => channel.id === autoNote.channel)?.channel
    : undefined;
  return (
    <section className={showInfo ? 'chat-screen with-info' : 'chat-screen'}>
      <ChatTeamColumn
        snapshot={snapshot}
        selected={ref}
        activeChannel={channel?.id}
        accessOptions={accessOptions}
        searchRef={search}
        onOpen={(next, channelId) => {
          // Remove the automatic-selection note after an explicit selection.
          setAutoOpened(null);
          onNavigate(
            channelId
              ? { kind: 'chat', ref: next, channel: channelId }
              : { kind: 'chat', ref: next },
          );
        }}
        // The column's button starts at the team step: the inbox spans every
        // team, so which one is open is not the answer to "new chat where".
        onNewChat={() => setNewChat({})}
        onSettings={(next) =>
          onNavigate({ kind: 'group-settings', ref: next, tab: 'settings' })
        }
        onCreateTeam={() => onNavigate({ kind: 'teams' })}
      />
      <section
        className="chat-conversation"
        aria-label="Conversation"
        tabIndex={-1}
        ref={conversation}
      >
        {autoNote && open && (
          <div className="chat-note">
            <span className="chat-note-glyph">
              <Icon name="info" size={15} />
            </span>
            <p>
              {autoNote.recent && autoChannel
                ? `Chat opened ${open.name} · ${channelTitle(autoChannel)} because no conversation was selected.`
                : `Chat opened ${open.name} because no team was selected.`}
            </p>
            <Button
              variant="quiet"
              icon="x"
              aria-label="Dismiss"
              title="Dismiss"
              onClick={() => {
                setNoteDismissed(true);
                // The dismissed note took focus with it; the conversation it
                // sat above takes it back.
                conversation.current?.focus();
              }}
            />
          </div>
        )}
        {ref ? (
          <ChatScreen
            key={ref}
            snapshot={snapshot}
            bridge={bridge}
            location={{ ...location, ref, channel: wanted }}
            onNavigate={onNavigate}
            accessNow={accessNow}
            accessOptions={accessOptions}
            // The generation belongs to the server of the team that is open,
            // which only the tab knows once it has resolved one.
            accessGeneration={accessGenerations.get(open?.server ?? '') ?? 0}
            infoOpen={info}
            onToggleInfo={() => setInfo((shown) => !shown)}
            infoRef={infoToggle}
            onNewChat={() => setNewChat({ team: ref })}
            onSearch={() => {
              search.current?.focus();
              search.current?.select();
            }}
          />
        ) : location.ref ? (
          <NoChatForTeam
            snapshot={snapshot}
            named={location.ref}
            accessOptions={accessOptions}
            onNavigate={onNavigate}
          />
        ) : opening === 'pending' ? (
          <div className="empty" aria-busy="true">
            <p role="status">Loading conversations…</p>
          </div>
        ) : (
          <NoTeamWithChat onNavigate={onNavigate} />
        )}
      </section>
      {showInfo && open && (
        <ChannelInfoPanel
          snapshot={snapshot}
          store={open}
          channel={channel}
          loading={loading}
          scope={entry?.scope}
          onNavigate={onNavigate}
          onClose={() => {
            setInfo(false);
            // During a loading switch the new pane may not have an info toggle.
            // Keep focus in the conversation instead of dropping it.
            (infoToggle.current ?? conversation.current)?.focus();
          }}
        />
      )}
      {newChat && (
        <NewChatSheet
          snapshot={snapshot}
          bridge={bridge}
          team={newChat.team}
          accessOptions={accessOptions}
          accessNow={accessNow}
          accessGenerations={accessGenerations}
          onUnresolved={(unresolved) => {
            newChatUnresolved.current = unresolved;
          }}
          onClose={() => {
            newChatUnresolved.current = false;
            setNewChat(null);
          }}
          onOpen={(next, channelId) => {
            newChatUnresolved.current = false;
            setNewChat(null);
            setAutoOpened(null);
            onNavigate(
              channelId
                ? { kind: 'chat', ref: next, channel: channelId }
                : { kind: 'chat', ref: next },
            );
          }}
        />
      )}
    </section>
  );
}

/** The pane for a location that names a team with no chat. */
function NoChatForTeam({
  snapshot,
  named,
  accessOptions,
  onNavigate,
}: {
  snapshot: AgentSnapshot;
  /** The store the location names, whether or not this Mac has it. */
  named: StoreRef;
  accessOptions?: AvailabilityOptions;
  onNavigate: (location: Location) => void;
}): ReactNode {
  const store = storeOf(snapshot, named);
  const team =
    store && store.kind === 'team' && store.team_kind === 'named'
      ? store
      : undefined;
  const reason = !store
    ? 'This team is not on this Mac.'
    : team
      ? `${noChatReason(snapshot, team)}.`
      : 'Chat is available in named teams whose server offers it. Vaults and shares never have chat.';
  const elsewhere = chatTeams(snapshot).find((candidate) =>
    chatAvailable(snapshot, candidate, accessOptions),
  );
  return (
    <div className="empty">
      <span className="big">
        <Icon name="chat" />
      </span>
      <h2>{store ? `${store.name} has no chat` : 'Team unavailable'}</h2>
      <p role="status">{reason}</p>
      {elsewhere && (
        <Button
          variant="primary"
          onClick={() => onNavigate({ kind: 'chat', ref: elsewhere.id })}
        >
          Open another team
        </Button>
      )}
    </div>
  );
}

/** The pane for a Mac where no team has chat at all. */
function NoTeamWithChat({
  onNavigate,
}: {
  onNavigate: (location: Location) => void;
}): ReactNode {
  return (
    <div className="empty">
      <span className="big">
        <Icon name="chat" />
      </span>
      <h2>No team chats yet</h2>
      <p>
        Chat is available in named teams whose server offers it. No team on this
        Mac has one.
      </p>
      <Button variant="primary" onClick={() => onNavigate({ kind: 'teams' })}>
        Create or join a team
      </Button>
      <div className="chat-how">
        <h3>How chat gets turned on</h3>
        <ol>
          <li>
            The store has to be a named team. Vaults and shares never have chat.
          </li>
          <li>The team&apos;s server has to offer chat.</li>
          <li>
            The team has to be active and readable by your role, and the
            server&apos;s check-in has to be current.
          </li>
        </ol>
      </div>
    </div>
  );
}
