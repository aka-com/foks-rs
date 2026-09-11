import { Fragment, useEffect, useId, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import {
  Band,
  Button,
  Chip,
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
import { CHAT_TEXT_BYTES } from '../chat-contract';
import type {
  ChatAction,
  ChatChannel,
  ChatConversation,
  ChatOperation,
  ChatReply,
} from '../chat-contract';
import { shortId, storeOf } from '../model';
import type { World } from '../model';
import type { Location } from '../location';
import { PageHeader } from '../shell/page-header';
import { failure, preparationCanChange, submissionId } from '../chat/actions';
import { useChatConversation } from '../chat/use-chat-conversation';
import { useChatHistory } from '../chat/use-chat-history';
import './chat.css';

const TEXT_LIMIT_LABEL = `${CHAT_TEXT_BYTES / 1024} KiB`;

function channelTitle(channel: ChatChannel): string {
  return `# ${channel.name || 'general'}`;
}

function accessSummary(channel: ChatChannel): string {
  if (!channel.readable) return 'Read access required';
  if (channel.read_role === channel.write_role)
    return `${channel.read_role} can read and write`;
  return `Read ${channel.read_role} · Write ${channel.write_role}`;
}

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
  world,
  bridge,
  location,
  onNavigate,
}: {
  world: World;
  bridge: Bridge;
  location: Extract<Location, { kind: 'team-chat' }>;
  onNavigate: (location: Location) => void;
}): ReactNode {
  const store = storeOf(world, location.ref);
  const {
    channels,
    conversations,
    pending,
    error,
    syncError,
    degraded,
    loading,
    blocked,
    revision,
    actor,
    request,
    refresh,
    refreshPending,
    markRead,
  } = useChatConversation(bridge, store?.server ?? '', location.ref);
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
  useEffect(() => {
    if (!created || !storeId) return;
    if (!channels.some((channel) => channel.id === created)) return;
    setCreated(null);
    onNavigate({ kind: 'team-chat', ref: storeId, channel: created });
  }, [channels, created, onNavigate, storeId]);
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
              Check the account and server, then reopen the conversation from
              the sidebar.
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
          <Button
            icon="file"
            onClick={() => onNavigate({ kind: 'store', ref: store.id })}
          >
            Files
          </Button>
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
                  onClick={() => void refresh()}
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
                      {conversationMeta(listedChannel, conversation)}
                    </small>
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
          {pending.length > 0 && (
            <section className="chat-recovery" aria-label="Needs attention">
              <SectionLabel>Needs attention</SectionLabel>
              <p className="chat-quiet">
                Saved work that has not finished. Nothing here is sent twice
                without your say-so.
              </p>
              {pending.map((op) => (
                <PendingRow
                  key={op.id}
                  operation={op}
                  channelName={channelNames.get(op.channel)}
                  request={request}
                  onChange={() => void refresh()}
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
                  onClick={() => void refresh()}
                >
                  Retry
                </Button>
              }
            >
              <span role="alert">{error}</span>
            </Band>
          )}
          {channel ? (
            <ChatThread
              key={channel.id}
              channel={channel}
              actor={actor}
              request={request}
              refreshPending={refreshPending}
              revision={revision}
              readThrough={activeConversation?.read_through ?? null}
              markRead={markRead}
            />
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
              <Button disabled={loading} onClick={() => void refresh()}>
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
          teamName={store.name}
          request={request}
          onClose={() => setCreating(false)}
          onCreated={async (channelId) => {
            setCreated(channelId);
            await refresh();
          }}
        />
      )}
    </section>
  );
}

function ChannelCreateSheet({
  teamName,
  request,
  onClose,
  onCreated,
}: {
  teamName: string;
  request: (a: ChatAction) => Promise<ChatReply>;
  onClose: () => void;
  onCreated: (channelId: string) => Promise<void>;
}): ReactNode {
  const [name, setName] = useState('');
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
      subtitle={`An encrypted channel in ${teamName}`}
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

const PENDING_STATE: Record<ChatOperation['state'], string> = {
  prepared: 'prepared',
  uncertain: 'Checking delivery',
  confirmed: 'confirmed',
  rejected: 'rejected',
  cancelled: 'cancelled',
};

function pendingExplanation(op: ChatOperation): string {
  switch (op.state) {
    case 'prepared':
      return 'Saved on this device and not sent yet.';
    case 'uncertain':
      return 'The server may already have it. Check delivery before sending anything again.';
    case 'confirmed':
      return 'Delivered. Finish cleanup to remove it from this list.';
    case 'cancelled':
      return 'Cancelled before it was sent.';
    case 'rejected':
      return `The server rejected this operation${
        op.rejection_code !== null ? ` (${op.rejection_code})` : ''
      }. Prepare a new message after resolving the error.`;
  }
}

function PendingRow({
  operation,
  channelName,
  request,
  onChange,
}: {
  operation: ChatOperation;
  channelName: string | undefined;
  request: (a: ChatAction) => Promise<ChatReply>;
  onChange: () => void;
}): ReactNode {
  const op = operation;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const active = useRef(true);
  const running = useRef(false);
  useEffect(() => {
    active.current = true;
    return () => {
      active.current = false;
    };
  }, []);
  const run = async (action: 'attempt' | 'cancel' | 'finalize') => {
    if (running.current) return;
    running.current = true;
    setBusy(true);
    setError('');
    try {
      const reply = await request({ action, operation: op.id });
      if (active.current && reply.result.kind === 'operation') {
        onChange();
      }
    } catch (e) {
      if (active.current) {
        setError(failure(e));
        try {
          await request({ action: 'status', operation: op.id });
        } catch {
          /* Retain the last known operation when status is unavailable. */
        }
      }
    } finally {
      running.current = false;
      if (active.current) setBusy(false);
    }
  };
  return (
    <div className="chat-pending" data-operation={op.id}>
      <span className="chat-pending-title">
        {op.create ? 'Channel' : 'Message'} · {PENDING_STATE[op.state]}
      </span>
      <small>
        {channelName ? `${channelName} · ` : ''}
        <span title={op.id}>{shortId(op.id)}</span>
      </small>
      <p>{pendingExplanation(op)}</p>
      <div className="chat-pending-actions">
        {(op.state === 'prepared' || op.state === 'uncertain') && (
          <Button size="sm" disabled={busy} onClick={() => void run('attempt')}>
            {op.state === 'uncertain' ? 'Check delivery' : 'Send prepared'}
          </Button>
        )}
        {op.state === 'prepared' && (
          <Button size="sm" disabled={busy} onClick={() => void run('cancel')}>
            Cancel preparation
          </Button>
        )}
        {['confirmed', 'rejected', 'cancelled'].includes(op.state) && (
          <Button
            size="sm"
            disabled={busy}
            onClick={() => void run('finalize')}
          >
            Finish cleanup
          </Button>
        )}
      </div>
      {error && (
        <p role="alert" className="action-error">
          {error}
        </p>
      )}
    </div>
  );
}

function ChatThread({
  channel,
  actor,
  request,
  refreshPending,
  revision,
  readThrough,
  markRead,
}: {
  channel: ChatChannel;
  actor: string | null;
  request: (a: ChatAction) => Promise<ChatReply>;
  refreshPending: () => Promise<void>;
  revision: number;
  readThrough: string | null;
  markRead: (channel: string, sequence: string) => Promise<void>;
}): ReactNode {
  const {
    messages,
    before,
    missing,
    error,
    setError,
    busy,
    load,
    scroller,
    active,
    atBottom,
    onScroll,
  } = useChatHistory(channel, request, revision);
  const [draft, setDraft] = useState('');
  const [sending, setSending] = useState(false);
  const [sendError, setSendError] = useState('');
  const [newFrom, setNewFrom] = useState<string | null>(null);
  const hintId = useId();
  const submission = useRef<Extract<
    ChatAction,
    { action: 'prepare-message' }
  > | null>(null);
  const sendGuard = useRef(false);
  const readMark = readThrough ?? '0';
  const markedThrough = useRef(readMark);
  const markingThrough = useRef('0');
  const draftBytes = useMemo(
    () => new TextEncoder().encode(draft).length,
    [draft],
  );
  useEffect(
    () => () => {
      submission.current = null;
    },
    [],
  );
  useEffect(() => {
    if (readThrough !== null) setNewFrom((old) => old ?? readThrough);
  }, [readThrough]);
  useEffect(() => {
    if (BigInt(readMark) > BigInt(markedThrough.current))
      markedThrough.current = readMark;
  }, [readMark]);
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const schedule = () => {
      if (timer) clearTimeout(timer);
      const latest = messages.at(-1)?.sequence;
      if (
        !latest ||
        !atBottom ||
        document.visibilityState === 'hidden' ||
        !document.hasFocus() ||
        BigInt(latest) <= BigInt(markedThrough.current) ||
        BigInt(latest) <= BigInt(markingThrough.current)
      )
        return;
      timer = setTimeout(() => {
        if (
          !atBottom ||
          document.visibilityState === 'hidden' ||
          !document.hasFocus() ||
          BigInt(latest) <= BigInt(markedThrough.current) ||
          BigInt(latest) <= BigInt(markingThrough.current)
        )
          return;
        markingThrough.current = latest;
        void markRead(channel.id, latest)
          .then(() => {
            markedThrough.current = latest;
          })
          .catch((cause) => setError(failure(cause)))
          .finally(() => {
            if (markingThrough.current === latest) markingThrough.current = '0';
          });
      }, 300);
    };
    schedule();
    window.addEventListener('focus', schedule);
    window.addEventListener('blur', schedule);
    document.addEventListener('visibilitychange', schedule);
    return () => {
      if (timer) clearTimeout(timer);
      window.removeEventListener('focus', schedule);
      window.removeEventListener('blur', schedule);
      document.removeEventListener('visibilitychange', schedule);
    };
  }, [atBottom, channel.id, markRead, messages, setError]);
  const jumpToLatest = () => {
    const element = scroller.current;
    if (!element) return;
    element.scrollTop = element.scrollHeight;
    onScroll();
  };
  const send = async () => {
    if (sendGuard.current || (!submission.current && !draft.trim())) return;
    if (draftBytes > CHAT_TEXT_BYTES) {
      setSendError(
        `Messages can contain up to ${TEXT_LIMIT_LABEL} of UTF-8 text.`,
      );
      return;
    }
    sendGuard.current = true;
    setSending(true);
    setSendError('');
    submission.current ??= {
      action: 'prepare-message',
      submission: submissionId(),
      channel: channel.id,
      text: draft,
    };
    let preparedId: string | null = null;
    try {
      const reply = await request(submission.current);
      if (!active.current) return;
      if (reply.result.kind !== 'operation')
        throw new Error('Invalid message preparation.');
      preparedId = reply.result.operation.id;
      submission.current = null;
      setDraft('');
      await refreshPending();
      if (!active.current) return;
      await request({
        action: 'attempt',
        operation: preparedId,
      });
      if (!active.current) return;
      await refreshPending();
      await load();
    } catch (e) {
      if (active.current) {
        if (submission.current && preparationCanChange(e))
          submission.current = null;
        setSendError(failure(e));
        if (preparedId) {
          try {
            await request({ action: 'status', operation: preparedId });
          } catch {
            /* Keep the durable identity visible if status is unavailable. */
          }
        }
        void refreshPending().catch(() => {});
      }
    } finally {
      sendGuard.current = false;
      if (active.current) setSending(false);
    }
  };
  const overLimit = draftBytes > CHAT_TEXT_BYTES;
  const nearLimit = draftBytes > CHAT_TEXT_BYTES * 0.75;
  const title = channelTitle(channel);
  return (
    <>
      <div className="chat-thread-header">
        <div className="chat-thread-title">
          <h2>{title}</h2>
          <small>{accessSummary(channel)}</small>
        </div>
        {channel.admin && <Chip>Admins</Chip>}
        {!channel.readable && <Chip tone="warn">Restricted</Chip>}
        <Button
          size="sm"
          icon="again"
          disabled={busy}
          onClick={() => {
            void load();
            void refreshPending().catch((e) => setError(failure(e)));
          }}
        >
          Refresh messages
        </Button>
      </div>
      {!channel.readable ? (
        <div className="empty">
          <span className="big">
            <Icon name="shield" />
          </span>
          <h2>Read access required</h2>
          <p>Your current role cannot read this channel.</p>
          <p>Ask a team admin to raise your role if you need to take part.</p>
        </div>
      ) : (
        <>
          {error && (
            <Band severity="crit">
              <span role="alert">{error}</span>
            </Band>
          )}
          {missing && (
            <Band>
              Some earlier messages could not be checked. This history has
              incomplete verification.
            </Band>
          )}
          <div className="chat-messages-wrap">
            <div
              className="chat-messages"
              ref={scroller}
              onScroll={onScroll}
              aria-label="Message history"
              aria-busy={busy}
            >
              <div className="chat-history-edge">
                {before ? (
                  <Button
                    size="sm"
                    disabled={busy}
                    onClick={() => void load(before)}
                  >
                    Load older messages
                  </Button>
                ) : messages.length > 0 ? (
                  <small>Beginning of the conversation</small>
                ) : null}
              </div>
              {!messages.length && !busy && !error && (
                <p className="chat-quiet chat-messages-empty">
                  No messages yet. Say hello to start the conversation.
                </p>
              )}
              {messages.map((m, index) => {
                const own = actor !== null && m.sender === actor;
                const isNew =
                  newFrom !== null &&
                  !own &&
                  BigInt(m.sequence) > BigInt(newFrom) &&
                  (index === 0 ||
                    BigInt(messages[index - 1].sequence) <= BigInt(newFrom));
                return (
                  <Fragment key={m.id}>
                    {isNew && (
                      <div
                        className="chat-divider"
                        role="separator"
                        aria-label="New messages"
                      >
                        <span>New</span>
                      </div>
                    )}
                    <article className="chat-message" data-message={m.id}>
                      <header>
                        <span
                          className={own ? 'chat-sender you' : 'chat-sender'}
                          title={m.sender ?? undefined}
                        >
                          {own
                            ? 'You'
                            : m.sender
                              ? shortId(m.sender)
                              : 'Team member'}
                        </span>
                        <small title={`Message ${m.sequence}`}>
                          #{m.sequence}
                        </small>
                      </header>
                      <p
                        className={
                          m.content.kind === 'text'
                            ? undefined
                            : 'chat-unsupported'
                        }
                      >
                        {m.content.kind === 'text'
                          ? m.content.text
                          : m.content.kind === 'oversized'
                            ? 'This message exceeds the desktop display limit.'
                            : 'This message type is not supported yet.'}
                      </p>
                    </article>
                  </Fragment>
                );
              })}
            </div>
            {!atBottom && messages.length > 0 && (
              <Button
                size="sm"
                icon="chev"
                className="chat-jump"
                onClick={jumpToLatest}
              >
                Jump to latest
              </Button>
            )}
          </div>
          <form
            className="chat-composer"
            onSubmit={(e) => {
              e.preventDefault();
              void send();
            }}
          >
            {sendError && (
              <Band severity="crit">
                <span role="alert">{sendError}</span>
              </Band>
            )}
            <textarea
              aria-label="Message"
              aria-describedby={hintId}
              value={draft}
              disabled={sending || submission.current !== null}
              placeholder={`Message ${title}`}
              rows={2}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => {
                if (
                  e.key === 'Enter' &&
                  !e.shiftKey &&
                  !e.nativeEvent.isComposing &&
                  e.keyCode !== 229
                ) {
                  e.preventDefault();
                  void send();
                }
              }}
            />
            <div className="chat-composer-row">
              <small id={hintId}>
                {submission.current
                  ? 'The reply to this message was lost. Recover it to send the same text once.'
                  : 'Enter to send · Shift+Enter for a new line'}
              </small>
              {nearLimit && (
                <small
                  className={overLimit ? 'chat-meter over' : 'chat-meter'}
                  aria-live="polite"
                >
                  {Math.ceil(draftBytes / 1024)} KiB of {TEXT_LIMIT_LABEL}
                </small>
              )}
              <Button
                variant="primary"
                type="submit"
                disabled={
                  sending || overLimit || (!draft.trim() && !submission.current)
                }
              >
                {sending
                  ? 'Sending…'
                  : submission.current
                    ? 'Recover preparation'
                    : 'Send'}
              </Button>
            </div>
          </form>
        </>
      )}
    </>
  );
}
