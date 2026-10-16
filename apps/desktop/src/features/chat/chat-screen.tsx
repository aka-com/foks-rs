import { ChatThread } from '../../chat/chat-thread';
import { channelAudience, directUser } from '../../chat/audience';
import { ChatConversationProvider } from '../../chat/conversation-provider';
import { chatMentions, mentionKey } from '../../chat/mentions';
import { useComposerAutofocus } from '../../chat/use-composer-autofocus';
import {
  ChatAlerts,
  failureAlert,
  type ChatAlert,
} from '../../chat/chat-alerts';
import { UnsentChip } from '../../chat/unsent-chip';
import { ThreadHeader } from '../../chat/thread-header';
import {
  channelTitle,
  listChannels,
  messageClock,
  openChannel,
  partyNames,
} from '../../chat/presentation';
import { chatDiagnostic } from '../../chat/diagnostic';
import { useOptionalToast } from '../../../kit/toasts';
import { useCallback, useEffect, useMemo, useState } from 'react';
import type { ReactNode, Ref } from 'react';
import { Button, Icon, Notice } from '../../components';
import type { Bridge } from '../../bridge';
import type { ChatAction, ChatReply } from '../../chat-contract';
import {
  chatAvailable,
  groupDetailFailure,
  partiesOf,
  serverDisplayLabelForStore,
  storeOperationAvailability,
  storeDescription,
  storeDescriptionState,
  storeOf,
} from '../../model';
import type { AgentSnapshot, AvailabilityOptions, StoreRef } from '../../model';
import type { Location } from '../../location';
import { useChatConversation } from '../../chat/use-chat-conversation';
import { lastPosition } from '../../chat/snapshots';
import { useChatSendService } from '../../chat/send-provider';
import { cancelled } from '../../chat/errors';
import { chatTeams } from './chat-teams';

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
  /** Opens the tab's Create channel sheet on this team. */
  onNewChat?: () => void;
}): ReactNode {
  const sends = useChatSendService();
  const autofocusComposer = useComposerAutofocus(location.channel);
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
    failure,
    syncError,
    note,
    degraded,
    loading,
    resyncing,
    blocked,
    blockedFailure,
    blockedChannels,
    channelRevisions,
    actor,
    scope,
    request,
    refresh,
    refreshPending,
    markRead,
    setOpenChannel,
    acceptHistory,
    history,
    blockHistory,
    recheckChannel,
    rechecked,
    channelBlockCause,
    listPosition,
    keepListPosition,
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
  // The inbox service exempts the presented channel from the backoff a
  // degraded projection is otherwise held to, so it has to be told which one
  // that is, and told again when this pane unmounts.
  const openId = channel?.id ?? null;
  useEffect(() => {
    setOpenChannel(openId);
    return () => setOpenChannel(null);
  }, [setOpenChannel, openId]);
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
  // This pane reads no draft while rendering, so typing does not re-render it.
  // A channel that is no longer listed cannot be returned to. Its draft is
  // dropped rather than held for an address that no longer resolves; an empty
  // list is only acted on once the channels are known.
  const channelIds = listed.map(({ channel }) => channel.id).join('\n');
  useEffect(() => {
    if (!channelsKnown) return;
    sends.retainDrafts(
      location.ref,
      new Set(channelIds ? channelIds.split('\n') : []),
    );
  }, [channelIds, sends, location.ref, channelsKnown]);
  const storeId = store?.id ?? '';
  // Resolve our own identity from the account even before the roster arrives.
  const ownName = agentSnapshot.accounts.find(
    (account) =>
      account.server === store?.server &&
      account.alias === (scope?.store.account_alias ?? store?.account),
  )?.username;
  // Built once per snapshot, so the conversation provider's value stays the
  // same across renders that change nothing it carries.
  const senderNames = useMemo(() => {
    const names = partyNames(agentSnapshot, storeId);
    if (actor && ownName) names.set(actor, ownName);
    return names;
  }, [agentSnapshot, storeId, actor, ownName]);
  // The roster's usernames, and the reader's own from the account or, failing
  // that, the roster.
  const mentions = useMemo(() => {
    const users = partiesOf(agentSnapshot, storeId).filter(
      (party) => party.party_kind === 'user',
    );
    return chatMentions(
      users.flatMap((party) => (party.username ? [party.username] : [])),
      ownName ?? users.find((party) => party.party_id_hex === actor)?.username,
    );
  }, [agentSnapshot, storeId, actor, ownName]);
  // The members `@name` completes to: the roster's direct users with a
  // username (the party set the info panel's reader list draws from), once
  // each, without the reader.
  const members = useMemo(() => {
    const seen = new Set(mentions.self ? [mentions.self] : []);
    return partiesOf(agentSnapshot, storeId).flatMap((party) => {
      const username = party.username;
      if (!directUser(party) || !username || party.party_id_hex === actor)
        return [];
      const key = mentionKey(username);
      if (seen.has(key)) return [];
      seen.add(key);
      return [{ username, role: party.destination_role }];
    });
  }, [agentSnapshot, storeId, actor, mentions.self]);
  // Who can read the open channel, from the roster the team page already
  // loads, read here rather than fetched again. `channelAudience` treats a
  // roster with no parties as not loaded, so the header omits the count
  // instead of showing zero. A failed roster load leaves whatever parties
  // were held; as in the info panel, no count is drawn from them.
  const audience = useMemo(
    () =>
      storeId &&
      channel &&
      !groupDetailFailure(agentSnapshot, storeId, 'roster')
        ? channelAudience(agentSnapshot, storeId, channel)
        : undefined,
    [agentSnapshot, storeId, channel],
  );
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
  const toasts = useOptionalToast();
  const copyDiagnostic = (stopped?: string) => {
    if (!store) return;
    // A stopped channel reports the failure that blocked it; a channel
    // `sync-inbox` listed as blocked has none, and says where it came from.
    const cause = stopped ? channelBlockCause(stopped) : blockedFailure;
    void bridge
      .copyText(
        chatDiagnostic({
          team: store.name,
          teamId: scope?.store.team_id,
          server: serverDisplayLabelForStore(agentSnapshot, store),
          channel: stopped,
          code: cause?.code ?? (stopped ? undefined : 'chat-integrity'),
          reportedBy:
            stopped && !cause ? 'sync-inbox blocked_channels' : undefined,
          detail: cause?.message,
          at: Date.now(),
        }),
      )
      .then(() => toasts?.show('Diagnostic details copied'))
      .catch(() => toasts?.show('Could not copy', { tone: 'warning' }));
  };
  // Whether this conversation has carried saved work since it was opened, so
  // the chip can say it was accounted for rather than simply vanishing.
  const [recovered, setRecovered] = useState(false);
  // The open thread draws its own channel's operations, except when the
  // channel is unreadable or stopped and no thread is drawn.
  const unfinished = pending.filter(
    (op) =>
      !op.observed &&
      (op.kind === 'create-channel' ||
        op.channel !== channel?.id ||
        !channel?.readable ||
        blockedChannels.has(op.channel)),
  );
  if (unfinished.length > 0 && !recovered) setRecovered(true);
  const readable = new Set(
    listed.flatMap(({ channel }) => (channel.readable ? [channel.id] : [])),
  );
  const attention = (
    <UnsentChip
      storeId={storeId}
      operations={unfinished}
      recovered={recovered}
      teamName={store?.name}
      channelName={(id) => channelNames.get(id)}
      canOpen={(id) => readable.has(id) && id !== channel?.id}
      onJump={(id) =>
        onNavigate({ kind: 'chat', ref: location.ref, channel: id })
      }
      onChange={() => void guardedRefresh()}
    />
  );
  // The header that carries the chip is drawn with a channel; without one, the
  // chip sits in a bar of its own above the pane.
  const headed =
    available && !blocked && Boolean(team) && channel !== undefined;
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
  const retryInbox = {
    label: 'Retry',
    disabled: loading,
    run: () => void guardedRefresh(),
  };
  // What a synchronization reports about the team as a whole, apart from the
  // read of the channel list itself: these stand wherever the pane does, since
  // a paused or partial synchronization is worth saying even when the list
  // could not be read at all.
  const syncAlerts: ChatAlert[] = [
    {
      message: syncError,
      severity: 'warn',
      label: 'Live updates paused',
      actions: [retryInbox],
    },
    // A synchronization that succeeded but could not finish everything
    // keeps updating; what it could not finish is a note, not a pause.
    { message: note, severity: 'info' },
    {
      message: degraded
        ? 'Some inbox changes could not be listed. Visible channels still refresh directly.'
        : '',
      severity: 'info',
    },
  ];
  // A failure over a channel list that was read, or over an open channel,
  // joins the thread's own alerts below its header. Without a channel,
  // the failure owns the empty pane instead of being repeated above it —
  // but only the failure: the synchronization alerts beside it stay.
  const alerts: ChatAlert[] = [
    failureAlert(failure, store?.name, [retryInbox]),
    ...syncAlerts,
  ];
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
          {serverDisplayLabelForStore(agentSnapshot, store)}.{' '}
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
                  section: 'account',
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
            Chat for this account stays stopped until a new chat session starts:
            lock and unlock FOKS to start one. Vault items and files are not
            affected.
          </p>
          <div className="chat-locked-actions">
            <Button onClick={() => copyDiagnostic()}>
              Copy diagnostic details
            </Button>
          </div>
        </Notice>
      </div>
    ) : !team ? (
      <div className="empty">
        <span className="big">
          <Icon name="users" />
        </span>
        <h2>Chat unavailable</h2>
        <p>Select an active team in the sidebar to open its conversations.</p>
      </div>
    ) : (
      <>
        {!channel && <ChatAlerts alerts={error ? syncAlerts : alerts} />}
        {channel && blockedChannels.has(channel.id) ? (
          <>
            <ThreadHeader
              channel={channel}
              audience={audience}
              onInfo={onToggleInfo}
              infoOpen={infoOpen}
              infoRef={infoRef}
              attention={attention}
            />
            <div className="empty">
              <span className="big">
                <Icon name="shield" />
              </span>
              <h2>Channel stopped</h2>
              <p role="alert">
                Content in {channelTitle(channel)} could not be verified, so its
                messages are not shown. Other channels remain available.
              </p>
              <p role="status">
                {rechecked.get(channel.id)?.stopped
                  ? `Checked again at ${messageClock(String(rechecked.get(channel.id)?.stopped))}. The content still could not be verified.`
                  : 'Recheck channel reads its history again and verifies it from the start.'}
              </p>
              <div className="chat-locked-actions">
                <Button
                  variant="primary"
                  onClick={() => recheckChannel(channel.id)}
                >
                  Recheck channel
                </Button>
                <Button onClick={() => copyDiagnostic(channel.id)}>
                  Copy diagnostic details
                </Button>
              </div>
            </div>
          </>
        ) : channel ? (
          <ChatConversationProvider
            bridge={bridge}
            storeId={storeId}
            actor={actor}
            scope={scope}
            senderNames={senderNames}
            mentions={mentions}
            members={members}
            request={guardedRequest}
            refreshPending={guardedRefreshPending}
            markRead={guardedMarkRead}
            acceptHistory={acceptHistory}
            blockHistory={blockHistory}
            listPosition={listPosition}
            keepListPosition={keepListPosition}
          >
            <ChatThread
              autofocusComposer={autofocusComposer}
              alerts={alerts}
              attention={attention}
              key={`${channel.id}:${channel.readable}`}
              channel={channel}
              teamName={store?.name}
              audience={audience}
              onInfo={onToggleInfo}
              infoOpen={infoOpen}
              infoRef={infoRef}
              revision={channelRevisions?.get(channel.id) ?? 0}
              // A degraded projection cannot state this channel's newest
              // position: its conversation rows stop advancing while the host
              // cannot report what changed, so gating the tail on them would
              // hold the open thread at whatever it last read. Withholding the
              // position is what keeps the open channel refreshing there; the
              // rate limit on the degraded invalidation bounds what that costs.
              position={
                activeConversation && !degraded
                  ? String(lastPosition(activeConversation))
                  : null
              }
              readThrough={activeConversation?.read_through ?? null}
              pendingRead={activeConversation?.pending_read ?? null}
              unread={activeConversation?.unread ?? null}
              history={history(channel.id)}
              incrementalHistory={
                agentSnapshot.agent.state === 'ready' &&
                agentSnapshot.agent.historyAfter === true
              }
              pending={pending.filter(
                (op) =>
                  op.kind !== 'create-channel' &&
                  !op.observed &&
                  op.channel === channel.id,
              )}
            />
          </ChatConversationProvider>
        ) : error ? (
          // The channel list could not be read, so there is nothing to say
          // about any one channel or about there being none: the failure is
          // the pane, with the one way out of it.
          <div className="empty">
            <span className="big">
              <Icon name="alert" />
            </span>
            <h2>Channels unavailable</h2>
            <p role="alert">{error}</p>
            <Button disabled={loading} onClick={() => void guardedRefresh()}>
              Retry
            </Button>
          </div>
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
              <Icon name="users" />
            </span>
            <h2>No conversations yet</h2>
            <p>Create a channel for this team to start chatting.</p>
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
      {!headed && (unfinished.length > 0 || recovered) && (
        <div className="chat-attention-bar">{attention}</div>
      )}
      {pane}
    </>
  );
}
