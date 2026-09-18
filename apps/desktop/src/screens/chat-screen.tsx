import { ChatThread } from '../chat/chat-thread';
import { PendingRow } from '../chat/pending-row';
import {
  channelTitle,
  listChannels,
  openChannel,
  partyNames,
} from '../chat/presentation';
import { useCallback, useEffect, useState } from 'react';
import type { ReactNode, Ref } from 'react';
import { Band, Button, Icon, Notice, SectionLabel } from '../components';
import type { Bridge } from '../bridge';
import type { ChatAction, ChatReply } from '../chat-contract';
import {
  chatAvailable,
  partiesOf,
  serverName,
  storeOperationAvailability,
  storeDescription,
  storeDescriptionState,
  storeOf,
} from '../model';
import type { AgentSnapshot, AvailabilityOptions, StoreRef } from '../model';
import type { Location } from '../location';
import { useChatConversation } from '../chat/use-chat-conversation';
import { useChatSends } from '../chat/send-provider';
import { cancelled } from '../chat/errors';
import { chatTeams } from './chat-teams';
import './chat.css';

const systemAccessNow = () => Date.now() / 1000;

/**
 * One team's conversation: the pane beside the Chat tab's team column. The tab
 * keys this on the open team, so a switch mounts exactly one of these.
 */
export function ChatScreen({
  snapshot: agentSnapshot,
  bridge,
  location,
  onNavigate,
  accessNow = systemAccessNow,
  accessOptions,
  accessGeneration = 0,
  infoOpen = false,
  onToggleInfo,
  infoRef,
  onNewChat,
  onSearch,
}: {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  location: Extract<Location, { kind: 'chat' }> & { ref: StoreRef };
  onNavigate: (location: Location) => void;
  accessNow?: () => number;
  accessOptions?: AvailabilityOptions;
  accessGeneration?: number;
  /** The tab owns the info panel, because it owns the layout it sits in. */
  infoOpen?: boolean;
  onToggleInfo?: () => void;
  /** The ⓘ toggle, so the tab can hand focus back when the panel closes. */
  infoRef?: Ref<HTMLButtonElement>;
  /** Opens the tab's New chat sheet on this team. */
  onNewChat?: () => void;
  /** Moves to the column's search field, the only search chat has. */
  onSearch?: () => void;
}): ReactNode {
  const { service: sends } = useChatSends();
  const store = storeOf(agentSnapshot, location.ref);
  const access = useCallback(
    () =>
      store
        ? storeOperationAvailability(agentSnapshot, store, 'chat', {
            ...accessOptions,
            nowSeconds: accessNow(),
          })
        : ({ available: false, reason: 'vault-unavailable' } as const),
    [accessNow, accessOptions, store, agentSnapshot],
  );
  const {
    channels,
    channelsKnown,
    conversations,
    pending,
    error,
    syncError,
    degraded,
    loading,
    resyncing,
    blocked,
    blockedChannels,
    channelRevisions,
    actor,
    scope,
    request,
    refresh,
    refreshPending,
    markRead,
    acceptHistory,
    history,
    blockHistory,
  } = useChatConversation(
    bridge,
    store?.server ?? '',
    location.ref,
    access,
    accessGeneration,
  );
  const listed = listChannels(channels, conversations);
  // The tab's column resolves the current row the same way, through the same
  // helper, so the row drawn as current is the channel this pane mounted.
  const channel = openChannel(listed, location.channel);
  const activeConversation = conversations.find(
    (conversation) => conversation.channel.id === channel?.id,
  );
  const channelNames = new Map(
    listed.map(({ channel }) => [channel.id, channelTitle(channel)]),
  );
  // Unsent messages are owned by the unlocked application, one per channel.
  // This pane only reconciles drafts against a completed channel listing.
  // Navigation can remount this pane without discarding any other draft.
  // Incomplete inbox data cannot establish that a channel was removed.
  // Lock and identity changes are handled by the application send service.
  const drafts = sends.drafts(location.ref);
  // A channel that is no longer listed cannot be returned to. Its draft is
  // dropped rather than held for an address that no longer resolves; an empty
  // list is only acted on once the channels are known.
  const channelIds = listed.map(({ channel }) => channel.id).join('\n');
  useEffect(() => {
    if (!channelsKnown) return;
    const live = new Set(channelIds ? channelIds.split('\n') : []);
    for (const id of [...drafts.keys()]) if (!live.has(id)) drafts.delete(id);
  }, [channelIds, drafts, channelsKnown]);
  const storeId = store?.id ?? '';
  const senderNames = partyNames(agentSnapshot, storeId);
  // The roster the team page already loads, read here rather than fetched
  // again. A team whose roster has not arrived yet has no parties on this
  // store id, which is indistinguishable from an empty team — but a real team
  // always has at least its own member, so an empty result means "not loaded"
  // and the header omits the count instead of showing zero.
  const memberCount = storeId
    ? partiesOf(agentSnapshot, storeId).length || undefined
    : undefined;
  const accessAvailable = useCallback(
    (): boolean => access().available,
    [access],
  );
  const guardedRequest = useCallback(
    (action: ChatAction): Promise<ChatReply> => request(action),
    [request],
  );
  const guardedRefresh = useCallback(
    (): Promise<void> => (accessAvailable() ? refresh() : Promise.resolve()),
    [accessAvailable, refresh],
  );
  const guardedRefreshPending = useCallback(
    (): Promise<void> =>
      accessAvailable() ? refreshPending() : Promise.resolve(),
    [accessAvailable, refreshPending],
  );
  const guardedMarkRead = useCallback(
    (channelId: string, sequence: string): Promise<void> =>
      accessAvailable()
        ? markRead(channelId, sequence)
        : Promise.reject(cancelled()),
    [accessAvailable, markRead],
  );
  const describeOptions = {
    ...(accessOptions ?? { nowSeconds: accessNow() }),
    operation: 'chat' as const,
  };
  const available = store
    ? storeOperationAvailability(agentSnapshot, store, 'chat', describeOptions)
        .available
    : false;
  // The reason the locked pane states is read off the clock the lock decision
  // used, so the two cannot disagree about a check-in that expired this second.
  const team =
    store &&
    store.kind === 'team' &&
    store.team_kind === 'named' &&
    store.active !== false
      ? store
      : undefined;
  const empty = !loading && !listed.length && !error;
  // Whether this conversation has carried saved work since it was opened, so
  // the section can say it was accounted for rather than simply vanishing.
  const [recovered, setRecovered] = useState(false);
  const unfinished = pending.filter(
    (op) =>
      !op.observed &&
      (op.kind === 'create-channel' ||
        op.channel !== channel?.id ||
        !channel?.readable),
  );
  if (unfinished.length > 0 && !recovered) setRecovered(true);
  // Only a team that can actually be opened is worth offering: `chatTeams`
  // filters on the server's capability, not on whether it answers today.
  const elsewhere = chatTeams(agentSnapshot).find(
    (candidate) =>
      candidate.id !== store?.id &&
      chatAvailable(agentSnapshot, candidate, describeOptions),
  );
  // A lapsed check-in is renewable here; every other unavailable state is
  // stated and left alone, because no button in Chat resolves it.
  const lapsed =
    store &&
    ['check-in-expired', 'check-in-unavailable'].includes(
      storeDescriptionState(agentSnapshot, store, describeOptions),
    );
  const pane =
    !available && store ? (
      // A lapsed check-in locks a whole server: the column stays, and this pane
      // says which server and where to renew it.
      <div className="chat-locked">
        <span className="big">
          <Icon name="shield" />
        </span>
        <h2>{store.name} chat is locked</h2>
        <p role="alert">
          {storeDescription(agentSnapshot, store, describeOptions)} on{' '}
          {serverName(agentSnapshot, store)}.{' '}
          {lapsed
            ? 'Every store on that server is unavailable until the server is checked again. Messages already on this device are kept.'
            : 'Messages already on this device are kept.'}
        </p>
        <div className="chat-locked-actions">
          {lapsed && (
            <Button
              variant="primary"
              onClick={() =>
                onNavigate({
                  kind: 'settings',
                  section: 'servers',
                  profile: store.server,
                })
              }
            >
              Check in
            </Button>
          )}
          {elsewhere && (
            <Button
              onClick={() => onNavigate({ kind: 'chat', ref: elsewhere.id })}
            >
              Open another team
            </Button>
          )}
        </div>
      </div>
    ) : blocked ? (
      <div className="chat-conversation-body">
        <Notice severity="crit" title="Chat stopped">
          <p role="alert">{blocked}</p>
          <p>
            Check the account and server connection, then lock and unlock FOKS
            to start a new chat session.
          </p>
        </Notice>
      </div>
    ) : !team ? (
      <div className="empty">
        <span className="big">
          <Icon name="people" />
        </span>
        <h2>Chat unavailable</h2>
        <p>Select an active team in the sidebar to open its conversations.</p>
      </div>
    ) : (
      <>
        {error && (
          <Band
            severity="crit"
            action={
              <Button
                size="sm"
                disabled={loading}
                onClick={() => void guardedRefresh()}
              >
                Retry
              </Button>
            }
          >
            <span role="alert">{error}</span>
          </Band>
        )}
        <div role="status" className="chat-status">
          {syncError && (
            <Band>
              Live updates paused
              <small>{syncError}</small>
            </Band>
          )}
          {degraded && (
            <Band severity="info">
              Some inbox changes could not be listed. Visible channels still
              refresh directly.
            </Band>
          )}
        </div>
        {channel && blockedChannels.has(channel.id) ? (
          <div className="empty">
            <h2>Channel stopped</h2>
            <p role="alert">
              Content in this channel could not be verified. Other channels
              remain available.
            </p>
            <p>
              Check the account and server connection, then lock and unlock FOKS
              to revalidate this channel.
            </p>
          </div>
        ) : channel ? (
          <ChatThread
            bridge={bridge}
            storeId={storeId}
            key={`${channel.id}:${channel.readable}`}
            channel={channel}
            teamName={team.name}
            memberCount={memberCount}
            onFiles={() => onNavigate({ kind: 'store', ref: team.id })}
            onSearch={onSearch}
            onInfo={onToggleInfo}
            infoOpen={infoOpen}
            infoRef={infoRef}
            actor={actor}
            scope={scope}
            senderNames={senderNames}
            request={guardedRequest}
            refreshPending={guardedRefreshPending}
            refreshInbox={guardedRefresh}
            revision={channelRevisions?.get(channel.id) ?? 0}
            readThrough={activeConversation?.read_through ?? null}
            markRead={guardedMarkRead}
            history={history}
            acceptHistory={acceptHistory}
            blockHistory={blockHistory}
            pending={pending.filter(
              (op) =>
                op.kind !== 'create-channel' &&
                !op.observed &&
                op.channel === channel.id,
            )}
          />
        ) : // Show a loading indicator if the requested channel is not yet listed
        // but the team is actively resynchronizing (such as immediately after
        // channel creation), avoiding premature 'unavailable' warnings.
        loading || (location.channel && resyncing) ? (
          <div
            className="app-loading"
            role="status"
            aria-label={`Loading ${team.name}`}
          >
            <span className="spin" aria-hidden="true" />
          </div>
        ) : location.channel ? (
          <div className="empty">
            <span className="big">
              <Icon name="alert" />
            </span>
            <h2>Channel unavailable</h2>
            <p>This channel is unavailable. Refresh to check your access.</p>
            <Button disabled={loading} onClick={() => void guardedRefresh()}>
              Refresh
            </Button>
          </div>
        ) : (
          <div className="empty">
            <span className="big">
              <Icon name="people" />
            </span>
            <h2>No conversations yet</h2>
            <p>
              Create the first channel for {team.name}. Every member with the
              right role can join in.
            </p>
            {empty && onNewChat && (
              <Button variant="primary" icon="plus" onClick={onNewChat}>
                New channel
              </Button>
            )}
          </div>
        )}
      </>
    );
  return (
    <>
      {unfinished.length > 0 ? (
        <section className="chat-recovery" aria-label="Needs attention">
          <SectionLabel>Needs attention</SectionLabel>
          <p className="chat-quiet">
            Saved work that has not finished. Nothing here is sent again unless
            you ask.
          </p>
          {unfinished.map((op) => (
            <PendingRow
              key={op.id}
              operation={op}
              channelName={channelNames.get(op.channel)}
              request={guardedRequest}
              onChange={() => void guardedRefresh()}
            />
          ))}
        </section>
      ) : (
        // What was saved here and then accounted for is worth saying once: the
        // section it sat in leaving without a word reads as work gone missing.
        recovered && (
          <section className="chat-recovery caught-up" aria-label="Saved work">
            <p className="chat-quiet" role="status">
              <Icon name="check" size={13} /> All caught up. Everything saved on
              this device has been accounted for.
            </p>
          </section>
        )
      )}
      {pane}
    </>
  );
}
