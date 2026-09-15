/**
 * The Chat tab: the team column beside one team's conversation.
 *
 * The column belongs to the tab, not to a conversation: it is built from the
 * store list and the inbox the rail already reads, so it survives a team
 * switch while exactly one conversation — the one the location names — is
 * mounted and unmounted beside it. `{kind:'chat'}` with no `ref` opens the
 * first team that has chat and says so, and says how chat gets turned on when
 * no team has any.
 */

import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { Button, Icon } from '../components';
import { chatAvailable, storeAvailability, storeOf } from '../model';
import type {
  AgentSnapshot,
  AvailabilityOptions,
  StoreRef,
  TeamStore,
} from '../model';
import type { Bridge } from '../bridge';
import type { Location } from '../location';
import { rememberChatLocation, rememberedChatRef } from '../location';
import { useSidebarInbox } from '../chat/inbox-provider';
import { listChannels, partyNames } from '../chat/presentation';
import { ChatScreen } from './chat-screen';
import { ChannelInfoPanel } from './chat-info';
import {
  ChatTeamColumn,
  chatTeams,
  firstChatTeam,
  noChatReason,
} from './chat-teams';
import './chat.css';

export interface ChatTabProps {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  location: Extract<Location, { kind: 'chat' }>;
  onNavigate: (location: Location) => void;
  accessNow?: () => number;
  accessGeneration?: number;
}

const systemAccessNow = () => Date.now() / 1000;

export function ChatTab({
  snapshot,
  bridge,
  location,
  onNavigate,
  accessNow = systemAccessNow,
  accessGeneration,
}: ChatTabProps): ReactNode {
  const inbox = useSidebarInbox();
  // One clock for the whole tab: the column, the pane and the fallback all
  // read availability off the moment this render started.
  const accessOptions: AvailabilityOptions = { nowSeconds: accessNow() };
  const teams = chatTeams(snapshot);
  const fallback = firstChatTeam(snapshot, accessOptions);
  // A team the location names but that has no chat is not opened: the pane
  // says why, and the column stays as it is.
  const named = location.ref
    ? teams.find((store) => store.id === location.ref)
    : undefined;
  const open: TeamStore | undefined = location.ref ? named : fallback;
  const ref: StoreRef | undefined = open?.id;
  const [autoOpened, setAutoOpened] = useState<StoreRef | null>(null);
  const [noteDismissed, setNoteDismissed] = useState(false);
  const [info, setInfo] = useState(false);
  const [creating, setCreating] = useState(false);
  // The ⓘ toggle takes focus back when the panel it opened closes, and the
  // conversation takes it when the note that was focused is dismissed.
  const infoToggle = useRef<HTMLButtonElement | null>(null);
  const conversation = useRef<HTMLElement | null>(null);
  // The tab with no team chosen is not a place to stay: it resolves to the
  // first team that has chat, and the location remembers the choice.
  useEffect(() => {
    if (!location.ref && fallback) {
      setAutoOpened(fallback.id);
      onNavigate({ kind: 'chat', ref: fallback.id });
    }
  }, [fallback, location.ref, onNavigate]);
  // A half-typed new-channel sheet belongs to the team it was opened in: a
  // switch — a notification activation, say — closes it rather than rebinding
  // it to the team that arrives.
  useEffect(() => setCreating(false), [ref]);
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
  const channel =
    listed.find(({ channel }) => channel.id === location.channel)?.channel ??
    (!location.channel ? listed[0]?.channel : undefined);
  const available = open
    ? storeAvailability(snapshot, open, accessOptions).available
    : false;
  const loading =
    Boolean(open) && (!entry || (entry.state === 'loading' && !entry.error));
  // The panel stays through a team switch: its channel arrives with the new
  // team's inbox, and the panel says so meanwhile rather than flickering out.
  const showInfo = info && Boolean(open);
  const autoNote =
    open && autoOpened === open.id && !noteDismissed ? open : undefined;
  return (
    <section className={showInfo ? 'chat-screen with-info' : 'chat-screen'}>
      <ChatTeamColumn
        snapshot={snapshot}
        selected={ref}
        channels={listed}
        activeChannel={channel?.id}
        blockedChannels={entry?.blockedChannels}
        actor={entry?.scope?.actor ?? null}
        senderNames={open ? partyNames(snapshot, open.id) : undefined}
        loading={loading}
        locked={Boolean(open) && !available}
        accessOptions={accessOptions}
        onSelectTeam={(next) => {
          // Remove the automatic-selection note after an explicit selection.
          setAutoOpened(null);
          onNavigate({ kind: 'chat', ref: next });
        }}
        onOpenChannel={(channelId) =>
          ref && onNavigate({ kind: 'chat', ref, channel: channelId })
        }
        onNewChannel={() => setCreating(true)}
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
        {autoNote && (
          <div className="chat-note">
            <span className="chat-note-glyph">
              <Icon name="info" size={15} />
            </span>
            <p>
              No team was named, so Chat opened {autoNote.name}, the first team
              with chat on this Mac.
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
            location={{ ...location, ref }}
            onNavigate={onNavigate}
            accessNow={accessNow}
            accessGeneration={accessGeneration}
            infoOpen={info}
            onToggleInfo={() => setInfo((shown) => !shown)}
            infoRef={infoToggle}
            creating={creating}
            onCreating={setCreating}
          />
        ) : location.ref ? (
          <NoChatForTeam
            snapshot={snapshot}
            named={location.ref}
            accessOptions={accessOptions}
            onNavigate={onNavigate}
          />
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
            // Closing hands focus back to the ⓘ that opened the panel rather
            // than dropping it on the document.
            infoToggle.current?.focus();
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
