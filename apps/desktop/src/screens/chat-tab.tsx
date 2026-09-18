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
import { listChannels, openChannel } from '../chat/presentation';
import { ChatScreen } from './chat-screen';
import { ChannelInfoPanel } from './chat-info';
import { NewChatSheet } from './chat-new';
import { useTabSheetState } from '../navigation-guard';
import { ChannelCreationCompletions } from '../chat/channel-creation-provider';
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
  const resolving = !location.ref;
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
  const [newChat, setNewChat] = useTabSheetState<{
    team?: StoreRef;
    originRef?: StoreRef;
    originChannel?: string;
  } | null>('chat.sheet', null);
  const [submittedSheet, setSubmittedSheet] = useState<typeof newChat>(null);
  const shownSheet = submittedSheet ?? newChat;
  const closeSheet = () => {
    setNewChat(null);
    setSubmittedSheet(null);
  };
  // The ⓘ toggle takes focus back when the panel it opened closes.
  const infoToggle = useRef<HTMLButtonElement | null>(null);
  const conversation = useRef<HTMLElement | null>(null);
  const search = useRef<HTMLInputElement | null>(null);
  // Unsubmitted sheets can resume from tab-session state. Submitted sheets
  // remain local views; closing or navigating never abandons the operation.
  // The tab with no conversation chosen is not a place to stay: it resolves to
  // the conversation with the most recent message, else to the first team that
  // has chat, and the location remembers the choice.
  useEffect(() => {
    if (location.ref || !openingRef) return;
    if (location.ref === openingRef && location.channel === openingChannel)
      return;
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
  // A New chat belongs to the location it was opened in: a team or channel
  // switch closes its view rather than rebinding it to the location arriving.
  // Unsubmitted rail-tab state restores on return; submitted work continues
  // in the controller without restoring an automatic redirect.
  const priorTeam = useRef(ref);
  const priorChannel = useRef(location.channel);
  useEffect(() => {
    if (
      priorTeam.current !== ref ||
      priorChannel.current !== location.channel
    ) {
      setNewChat(null);
      setSubmittedSheet(null);
    }
    priorTeam.current = ref;
    priorChannel.current = location.channel;
  }, [ref, location.channel, setNewChat]);
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
  const wanted = location.ref ? location.channel : openingChannel;
  const channel = wanted ? openChannel(listed, wanted) : undefined;
  const loading =
    Boolean(open) && (!entry || (entry.state === 'loading' && !entry.error));
  useEffect(() => {
    if (info && open && !loading && !channel) setInfo(false);
  }, [channel, info, loading, open]);
  // The panel stays through a team switch: its channel arrives with the new
  // team's inbox, and the panel says so meanwhile rather than flickering out.
  const showInfo = info && Boolean(open);
  return (
    <section className={showInfo ? 'chat-screen with-info' : 'chat-screen'}>
      <ChatTeamColumn
        snapshot={snapshot}
        selected={ref}
        activeChannel={channel?.id}
        accessOptions={accessOptions}
        searchRef={search}
        onOpen={(next, channelId) => {
          onNavigate(
            channelId
              ? { kind: 'chat', ref: next, channel: channelId }
              : { kind: 'chat', ref: next },
          );
        }}
        // The column's button searches conversations across every team;
        // creating a channel is a separate form with its own team choice.
        onNewChat={() =>
          setNewChat({ originRef: ref, originChannel: location.channel })
        }
        onSettings={(next) =>
          onNavigate({ kind: 'group-settings', ref: next, tab: 'settings' })
        }
        onTeams={(next) => onNavigate({ kind: 'teams', store: next })}
      />
      <section
        className="chat-conversation"
        aria-label="Conversation"
        tabIndex={-1}
        ref={conversation}
      >
        <ChannelCreationCompletions
          onOpen={(next, channelId) =>
            onNavigate({ kind: 'chat', ref: next, channel: channelId })
          }
        />
        {ref &&
        !wanted &&
        open &&
        chatAvailable(snapshot, open, accessOptions) ? (
          <div className="empty" role="status">
            <p>Choose a channel to open a conversation.</p>
          </div>
        ) : ref ? (
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
            onNewChat={() =>
              setNewChat({
                team: ref,
                originRef: ref,
                originChannel: location.channel,
              })
            }
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
      {shownSheet &&
        shownSheet.originRef === ref &&
        shownSheet.originChannel === location.channel && (
          <NewChatSheet
            snapshot={snapshot}
            bridge={bridge}
            team={shownSheet.team}
            accessOptions={accessOptions}
            accessNow={accessNow}
            accessGenerations={accessGenerations}
            onSubmitted={() => {
              setSubmittedSheet(shownSheet);
              setNewChat(null);
            }}
            onDraft={() => {
              setNewChat(shownSheet);
              setSubmittedSheet(null);
            }}
            onClose={closeSheet}
            onOpen={(next, channelId) => {
              closeSheet();
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
    ? 'This team is not on this device.'
    : team
      ? `${noChatReason(snapshot, team)}.`
      : 'Chat is only available in teams whose server supports it. Personal vaults and shares do not include chat.';
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
        Chat is available in teams when supported by their server. Create or
        join a team to get started.
      </p>
      <Button variant="primary" onClick={() => onNavigate({ kind: 'teams' })}>
        Create or join a team
      </Button>
    </div>
  );
}
