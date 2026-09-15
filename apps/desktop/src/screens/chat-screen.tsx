import { useChatInbox } from '../chat/inbox-provider';
import { NotificationSettings } from '../chat/notification-provider';
import { ChatThread } from '../chat/chat-thread';
import { PendingRow } from '../chat/pending-row';
import { channelTitle } from '../chat/presentation';
import { useCallback, useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import {
  Band,
  Button,
  Icon,
  Inset,
  InsetRow,
  Notice,
  RadioCard,
  RadioGroup,
  SectionLabel,
  SheetDialog,
} from '../components';
import type { Bridge } from '../bridge';
import type {
  ChatAction,
  ChatChannel,
  ChatConversation,
  ChatReply,
} from '../chat-contract';
import {
  partiesOf,
  shortId,
  storeAvailability,
  storeDescription,
  storeOf,
} from '../model';
import type { AgentSnapshot } from '../model';
import type { Location } from '../location';
import { PageHeader } from '../shell/page-header';
import { failure, preparationCanChange, submissionId } from '../chat/actions';
import { useChatConversation } from '../chat/use-chat-conversation';
import './chat.css';

const systemAccessNow = () => Date.now() / 1000;

function conversationMeta(
  channel: ChatChannel,
  conversation: ChatConversation | undefined,
): string {
  return [
    channel.admin ? 'Admins' : 'Team',
    !channel.readable ? 'Restricted' : '',
    conversation?.muted ? 'Muted' : '',
  ]
    .filter(Boolean)
    .join(' · ');
}

export function ChatScreen({
  snapshot: agentSnapshot,
  bridge,
  location,
  onNavigate,
  accessNow = systemAccessNow,
  accessGeneration = 0,
}: {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  location: Extract<Location, { kind: 'team-chat' }>;
  onNavigate: (location: Location) => void;
  accessNow?: () => number;
  accessGeneration?: number;
}): ReactNode {
  const store = storeOf(agentSnapshot, location.ref);
  const { snapshot } = useChatInbox();
  const access = useCallback(
    () =>
      store
        ? storeAvailability(agentSnapshot, store, { nowSeconds: accessNow() })
        : ({ available: false, reason: 'vault-unavailable' } as const),
    [accessNow, store, agentSnapshot],
  );
  const {
    channels,
    conversations,
    pending,
    error,
    syncError,
    degraded,
    loading,
    blocked,
    blockedChannels,
    channelRevisions,
    actor,
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
  const [creating, setCreating] = useState(false);
  const [created, setCreated] = useState<string | null>(null);
  const currentChannels = new Map(
    channels.map((channel) => [channel.id, channel]),
  );
  const conversationIds = new Set(
    conversations.map((conversation) => conversation.channel.id),
  );
  const listed = [
    ...conversations
      .filter((conversation) => !conversation.hidden)
      .map((conversation) => ({
        channel:
          currentChannels.get(conversation.channel.id) ?? conversation.channel,
        conversation,
      })),
    ...channels
      .filter((channel) => !conversationIds.has(channel.id))
      .map((channel) => ({ channel, conversation: undefined })),
  ];
  const channel =
    listed.find(({ channel }) => channel.id === location.channel)?.channel ??
    (!location.channel ? listed[0]?.channel : undefined);
  const activeConversation = conversations.find(
    (conversation) => conversation.channel.id === channel?.id,
  );
  const channelNames = new Map(
    listed.map(({ channel }) => [channel.id, channelTitle(channel)]),
  );
  const storeId = store?.id ?? '';
  const senderNames = new Map(
    store
      ? partiesOf(agentSnapshot, store.id)
          .filter((party) => party.party_kind === 'user')
          .map((party) => [
            party.party_id_hex,
            party.username ?? party.label ?? shortId(party.party_id_hex),
          ])
      : [],
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
      accessAvailable() ? markRead(channelId, sequence) : Promise.resolve(),
    [accessAvailable, markRead],
  );
  useEffect(() => {
    if (!created || !storeId) return;
    if (!channels.some((channel) => channel.id === created)) return;
    setCreated(null);
    onNavigate({ kind: 'team-chat', ref: storeId, channel: created });
  }, [channels, created, onNavigate, storeId]);
  if (!accessAvailable() && store)
    return (
      <section className="chat-screen">
        <PageHeader
          title="Chat unavailable"
          subtitle={storeDescription(agentSnapshot, store)}
        />
        <div className="chat-conversation">
          <Notice
            severity="crit"
            title={storeDescription(agentSnapshot, store)}
          >
            <p role="alert">
              Access to this group is stopped. Check the server status before
              reopening its conversations.
            </p>
          </Notice>
        </div>
      </section>
    );
  if (blocked)
    return (
      <section className="chat-screen">
        <PageHeader
          title="Chat stopped"
          subtitle="This conversation view is no longer trusted."
        />
        <div className="chat-conversation">
          <Notice severity="crit" title="Chat stopped">
            <p role="alert">{blocked}</p>
            <p>
              Check the account and server, then lock and unlock the desktop to
              start a fresh chat session.
            </p>
          </Notice>
        </div>
      </section>
    );
  if (
    !store ||
    store.kind !== 'team' ||
    store.team_kind !== 'named' ||
    store.active === false
  )
    return (
      <section className="chat-screen">
        <PageHeader
          title="Chat unavailable"
          subtitle="Chat works in active named teams."
        />
        <div className="chat-conversation">
          <div className="empty">
            <span className="big">
              <Icon name="people" />
            </span>
            <h2>Chat unavailable</h2>
            <p>
              Select an active named team in the sidebar to open its
              conversations.
            </p>
          </div>
        </div>
      </section>
    );
  const openCreate = () => setCreating(true);
  const empty = !loading && !listed.length && !error;
  return (
    <section className="chat-screen">
      <PageHeader
        title={`${store.name} · Chat`}
        subtitle="Encrypted channels for team members"
        action={
          <>
            <Button
              icon="people"
              title="Manage membership and access to team files and chat"
              onClick={() =>
                onNavigate({
                  kind: 'group-settings',
                  ref: store.id,
                  tab: 'people',
                })
              }
            >
              Team members
            </Button>
            <Button
              icon="file"
              onClick={() => onNavigate({ kind: 'store', ref: store.id })}
            >
              Files
            </Button>
          </>
        }
      />
      <div className="chat-layout">
        <aside className="chat-channels" aria-label="Conversations">
          <SectionLabel
            action={
              <>
                <Button
                  variant="quiet"
                  icon="again"
                  aria-label="Refresh conversations"
                  title="Refresh conversations"
                  disabled={loading}
                  onClick={() => void guardedRefresh()}
                />
                <Button
                  variant="quiet"
                  icon="plus"
                  aria-label="New channel"
                  title="New channel"
                  onClick={openCreate}
                />
              </>
            }
          >
            Conversations
          </SectionLabel>
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
          <div className="chat-channel-list">
            {listed.map(({ channel: listedChannel, conversation }) => {
              const active = channel?.id === listedChannel.id;
              const unread = conversation
                ? BigInt(conversation.unread) > 0n
                : false;
              return (
                <button
                  type="button"
                  className={[
                    'nav',
                    'chat-channel',
                    active ? 'on' : '',
                    unread ? 'unread' : '',
                  ]
                    .filter(Boolean)
                    .join(' ')}
                  aria-current={active ? 'page' : undefined}
                  key={listedChannel.id}
                  onClick={() =>
                    onNavigate({
                      kind: 'team-chat',
                      ref: store.id,
                      channel: listedChannel.id,
                    })
                  }
                >
                  <span className="t">
                    <span className="chat-channel-name">
                      {channelTitle(listedChannel)}
                    </span>
                    <small>
                      {blockedChannels.has(listedChannel.id)
                        ? 'Verification stopped'
                        : conversationMeta(listedChannel, conversation)}
                    </small>
                    {conversation?.preview && (
                      <small className="chat-preview">
                        {conversation.preview.sender === actor
                          ? 'You'
                          : conversation.preview.sender
                            ? (senderNames.get(conversation.preview.sender) ??
                              shortId(conversation.preview.sender))
                            : 'Team member'}
                        {': '}
                        {conversation.preview.content.kind === 'text'
                          ? conversation.preview.content.text
                          : 'Unsupported message'}
                      </small>
                    )}
                  </span>
                  {conversation && unread && (
                    <span
                      className={
                        conversation.muted ? 'chat-unread muted' : 'chat-unread'
                      }
                      aria-label={`${conversation.unread} unread`}
                    >
                      {conversation.unread}
                    </span>
                  )}
                </button>
              );
            })}
          </div>
          {loading && !listed.length && (
            <p className="chat-quiet">Loading conversations…</p>
          )}
          {empty && (
            <p className="chat-quiet">
              No channels yet. Create the first one to start talking.
            </p>
          )}
          {pending.some(
            (op) =>
              !op.observed &&
              (op.kind === 'create-channel' ||
                op.channel !== channel?.id ||
                !channel?.readable),
          ) && (
            <section className="chat-recovery" aria-label="Needs attention">
              <SectionLabel>Needs attention</SectionLabel>
              <p className="chat-quiet">
                Saved work that has not finished. Nothing here is sent twice
                without your say-so.
              </p>
              {pending
                .filter(
                  (op) =>
                    !op.observed &&
                    (op.kind === 'create-channel' ||
                      op.channel !== channel?.id ||
                      !channel?.readable),
                )
                .map((op) => (
                  <PendingRow
                    key={op.id}
                    operation={op}
                    channelName={channelNames.get(op.channel)}
                    request={guardedRequest}
                    onChange={() => void guardedRefresh()}
                  />
                ))}
            </section>
          )}
        </aside>
        <div className="chat-conversation">
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
          {channel && blockedChannels.has(channel.id) ? (
            <div className="empty">
              <h2>Channel stopped</h2>
              <p role="alert">
                Content in this channel could not be verified. Other channels
                remain available.
              </p>
              <p>
                Check the account and server, then lock and unlock the desktop
                to revalidate this channel.
              </p>
            </div>
          ) : channel ? (
            <>
              <NotificationSettings
                storeId={storeId}
                scope={snapshot.get(storeId)?.scope}
                channel={channel.id}
              />
              <ChatThread
                bridge={bridge}
                storeId={storeId}
                key={`${channel.id}:${channel.readable}`}
                channel={channel}
                actor={actor}
                senderNames={senderNames}
                request={guardedRequest}
                refreshPending={guardedRefreshPending}
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
            </>
          ) : loading ? (
            <div className="empty" aria-busy="true">
              <p>Loading conversations…</p>
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
                Create the first channel for {store.name}. Every member with the
                right role can join in.
              </p>
              {empty && (
                <Button variant="primary" icon="plus" onClick={openCreate}>
                  New channel
                </Button>
              )}
            </div>
          )}
        </div>
      </div>
      {creating && (
        <ChannelCreateSheet
          request={guardedRequest}
          onClose={() => setCreating(false)}
          onCreated={async (channelId) => {
            setCreated(channelId);
            await guardedRefresh();
          }}
        />
      )}
    </section>
  );
}

function ChannelCreateSheet({
  request,
  onClose,
  onCreated,
}: {
  request: (a: ChatAction) => Promise<ChatReply>;
  onClose: () => void;
  onCreated: (channelId: string) => Promise<void>;
}): ReactNode {
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [admin, setAdmin] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const submission = useRef<ChatAction | null>(null);
  const sending = useRef(false);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      submission.current = null;
    };
  }, []);
  const locked = busy || submission.current !== null;
  const create = async () => {
    if (sending.current) return;
    sending.current = true;
    setBusy(true);
    setError('');
    submission.current ??= {
      action: 'prepare-channel',
      submission: submissionId(),
      name,
      description,
      admin,
    };
    let preparedId: string | null = null;
    try {
      const prepared = await request(submission.current);
      if (!alive.current) return;
      if (prepared.result.kind !== 'operation')
        throw new Error('Invalid channel preparation.');
      const op = prepared.result.operation;
      preparedId = op.id;
      submission.current = null;
      setName('');
      setDescription('');
      const attempted = await request({ action: 'attempt', operation: op.id });
      if (!alive.current) return;
      const channelId =
        attempted.result.kind === 'operation'
          ? attempted.result.operation.channel
          : op.channel;
      await onCreated(channelId);
      if (alive.current) onClose();
    } catch (e) {
      if (alive.current) {
        if (submission.current && preparationCanChange(e))
          submission.current = null;
        setError(failure(e));
        if (preparedId) {
          try {
            await request({ action: 'status', operation: preparedId });
          } catch {
            /* Keep the durable identity visible if status is unavailable. */
          }
        }
      }
    } finally {
      sending.current = false;
      if (alive.current) setBusy(false);
    }
  };
  return (
    <SheetDialog
      title="New channel"
      onClose={onClose}
      dismissible={!locked}
      footer={
        <>
          <Button disabled={locked} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            disabled={busy}
            onClick={() => void create()}
          >
            {busy
              ? 'Creating…'
              : submission.current
                ? 'Recover preparation'
                : 'Create channel'}
          </Button>
        </>
      }
    >
      <form
        className="chat-create"
        onSubmit={(e) => {
          e.preventDefault();
          void create();
        }}
      >
        <Inset>
          <InsetRow label="Channel name">
            <input
              aria-label="Channel name"
              data-sheet-autofocus="true"
              value={name}
              disabled={locked}
              onChange={(e) => setName(e.target.value)}
              placeholder="design"
              maxLength={32}
            />
            <small>
              3–32 characters, lowercased automatically. Leave it empty only for
              the general channel.
            </small>
          </InsetRow>
          <InsetRow label="Description">
            <textarea
              aria-label="Channel description"
              value={description}
              disabled={locked}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="What this channel is for"
              maxLength={1024}
            />
            <small>
              Optional, 3–512 characters. Descriptions are lowercased
              automatically.
            </small>
          </InsetRow>
        </Inset>
        <SectionLabel>Who can take part</SectionLabel>
        <Inset>
          <RadioGroup label="Channel audience">
            <RadioCard
              title="Everyone on the team"
              detail="Members and above can read and write."
              selected={!admin}
              disabled={locked}
              onSelect={() => setAdmin(false)}
            />
            <RadioCard
              title="Admins and owners"
              detail="Hidden from members. Only admins and owners can read or write."
              selected={admin}
              disabled={locked}
              onSelect={() => setAdmin(true)}
            />
          </RadioGroup>
        </Inset>
        {submission.current && !busy && (
          <p className="hint">
            The server reply was lost. Recover retries the same request so the
            channel is not created twice.
          </p>
        )}
        {error && (
          <p role="alert" className="action-error">
            {error}
          </p>
        )}
      </form>
    </SheetDialog>
  );
}
