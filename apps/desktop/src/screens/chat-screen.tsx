import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { Button } from '../components';
import type { Bridge } from '../bridge';
import { CHAT_TEXT_BYTES } from '../chat-contract';
import type {
  ChatAction,
  ChatChannel,
  ChatOperation,
  ChatReply,
} from '../chat-contract';
import { storeOf } from '../model';
import type { World } from '../model';
import type { Location } from '../location';
import { PageHeader } from '../shell/page-header';
import { failure, preparationCanChange, submissionId } from '../chat/actions';
import { useChatConversation } from '../chat/use-chat-conversation';
import { useChatHistory } from '../chat/use-chat-history';
import './chat.css';

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
    pending,
    error,
    loading,
    blocked,
    request,
    refresh,
    refreshPending,
  } = useChatConversation(bridge, store?.server ?? '', location.ref);
  const channel =
    channels.find((c) => c.id === location.channel) ??
    (!location.channel ? channels[0] : undefined);
  if (blocked)
    return (
      <section className="chat-screen">
        <PageHeader
          title="Chat stopped"
          subtitle="Reopen the conversation after checking the account."
        />
        <p role="alert" className="chat-error">
          {blocked}
        </p>
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
          subtitle="Select an active named team."
        />
      </section>
    );
  return (
    <section className="chat-screen">
      <PageHeader
        title={`${store.name} · Chat`}
        subtitle="Encrypted team conversations"
        action={
          <Button onClick={() => onNavigate({ kind: 'store', ref: store.id })}>
            Files
          </Button>
        }
      />
      <div className="chat-layout">
        <aside className="chat-channels" aria-label="Channels">
          <div className="chat-row">
            <strong>Channels</strong>
            <Button size="sm" disabled={loading} onClick={() => void refresh()}>
              Refresh
            </Button>
          </div>
          {channels.map((c) => (
            <button
              type="button"
              className="chat-channel"
              aria-current={channel?.id === c.id ? 'page' : undefined}
              key={c.id}
              onClick={() =>
                onNavigate({ kind: 'team-chat', ref: store.id, channel: c.id })
              }
            >
              # {c.name || 'general'}
              <small>
                {c.admin ? 'Admins' : 'Team'}
                {!c.readable ? ' · Restricted' : ''}
              </small>
            </button>
          ))}
          {!loading && !channels.length && !error && (
            <p>No channels yet. Create the first conversation.</p>
          )}
          <ChannelCreate request={request} onCreated={refresh} />
          <strong>Operation recovery</strong>
          {pending.map((op) => (
            <PendingRow
              key={op.id}
              operation={op}
              request={request}
              onChange={() => void refresh()}
            />
          ))}
        </aside>
        <div className="chat-conversation">
          {error && (
            <p role="alert" className="chat-error">
              {error}
            </p>
          )}
          {channel ? (
            <ChatThread
              key={channel.id}
              channel={channel}
              request={request}
              refreshPending={refreshPending}
            />
          ) : (
            <p className="chat-empty">
              {loading
                ? 'Loading channels…'
                : location.channel
                  ? 'This channel is unavailable. Refresh to check your access.'
                  : 'Choose or create a channel.'}
            </p>
          )}
        </div>
      </div>
    </section>
  );
}

function ChannelCreate({
  request,
  onCreated,
}: {
  request: (a: ChatAction) => Promise<ChatReply>;
  onCreated: () => Promise<void>;
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
    try {
      const prepared = await request(submission.current);
      if (!alive.current) return;
      if (prepared.result.kind !== 'operation')
        throw new Error('Invalid channel preparation.');
      const op = prepared.result.operation;
      submission.current = null;
      setName('');
      await request({ action: 'attempt', operation: op.id });
      if (alive.current) await onCreated();
    } catch (e) {
      if (alive.current) {
        if (submission.current && preparationCanChange(e))
          submission.current = null;
        setError(failure(e));
      }
    } finally {
      sending.current = false;
      if (alive.current) setBusy(false);
    }
  };
  return (
    <form
      className="chat-create"
      onSubmit={(e) => {
        e.preventDefault();
        void create();
      }}
    >
      <label>
        New channel
        <input
          aria-label="Channel name"
          value={name}
          disabled={busy || submission.current !== null}
          onChange={(e) => setName(e.target.value)}
          placeholder="general (leave empty)"
          maxLength={128}
        />
      </label>
      <label>
        Audience
        <select
          aria-label="Channel audience"
          value={admin ? 'admin' : 'team'}
          disabled={busy || submission.current !== null}
          onChange={(e) => setAdmin(e.target.value === 'admin')}
        >
          <option value="team">Members and above</option>
          <option value="admin">Admins and owners</option>
        </select>
      </label>
      <Button type="submit" size="sm" disabled={busy}>
        {busy
          ? 'Creating…'
          : submission.current
            ? 'Recover preparation'
            : 'Create channel'}
      </Button>
      {error && (
        <p role="alert" className="chat-error">
          {error}
        </p>
      )}
    </form>
  );
}
function PendingRow({
  operation,
  request,
  onChange,
}: {
  operation: ChatOperation;
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
      <span>
        {op.create ? 'Channel' : 'Message'} ·{' '}
        {op.state === 'uncertain' ? 'Checking delivery' : op.state}
      </span>
      <small title={op.id}>{op.id.slice(0, 12)}</small>
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
      {op.state === 'rejected' && (
        <p>
          The server rejected this operation
          {op.rejection_code !== null ? ` (${op.rejection_code})` : ''}. Prepare
          a new message after resolving the error.
        </p>
      )}
      {error && (
        <p role="alert" className="chat-error">
          {error}
        </p>
      )}
      {['confirmed', 'rejected', 'cancelled'].includes(op.state) && (
        <Button size="sm" disabled={busy} onClick={() => void run('finalize')}>
          Finish cleanup
        </Button>
      )}
    </div>
  );
}
function ChatThread({
  channel,
  request,
  refreshPending,
}: {
  channel: ChatChannel;
  request: (a: ChatAction) => Promise<ChatReply>;
  refreshPending: () => Promise<void>;
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
  } = useChatHistory(channel, request);
  const [draft, setDraft] = useState('');
  const [sending, setSending] = useState(false);
  const submission = useRef<Extract<
    ChatAction,
    { action: 'prepare-message' }
  > | null>(null);
  const sendGuard = useRef(false);
  useEffect(
    () => () => {
      submission.current = null;
    },
    [],
  );
  const send = async () => {
    if (sendGuard.current || (!submission.current && !draft.trim())) return;
    if (new TextEncoder().encode(draft).length > CHAT_TEXT_BYTES) {
      setError('Messages can contain up to 64 KiB of UTF-8 text.');
      return;
    }
    sendGuard.current = true;
    setSending(true);
    setError('');
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
      await request({ action: 'attempt', operation: preparedId });
      if (!active.current) return;
      await refreshPending();
      await load();
    } catch (e) {
      if (active.current) {
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
        void refreshPending().catch(() => {});
      }
    } finally {
      sendGuard.current = false;
      if (active.current) setSending(false);
    }
  };
  return (
    <>
      <div className="chat-thread-header">
        <div>
          <strong># {channel.name || 'general'}</strong>
          <small>
            Read: {channel.read_role} · Write: {channel.write_role}
          </small>
        </div>
        <Button
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
        <p className="chat-empty">
          Your current role cannot read this channel.
        </p>
      ) : (
        <>
          {error && (
            <p role="alert" className="chat-error">
              {error}
            </p>
          )}
          {missing && (
            <p className="chat-notice">
              Some earlier messages could not be checked. This history has
              incomplete verification.
            </p>
          )}
          <div
            className="chat-messages"
            ref={scroller}
            aria-label="Message history"
            aria-busy={busy}
          >
            {before && (
              <Button
                size="sm"
                disabled={busy}
                onClick={() => void load(before)}
              >
                Load older messages
              </Button>
            )}
            {!messages.length && !busy && !error && <p>No messages yet.</p>}
            {messages.map((m) => (
              <article className="chat-message" key={m.id} data-message={m.id}>
                <header>
                  <span title={m.sender ?? undefined}>
                    {m.sender ? m.sender.slice(0, 14) : 'Team member'}
                  </span>
                  <small>#{m.sequence}</small>
                </header>
                <p>
                  {m.content.kind === 'text'
                    ? m.content.text
                    : m.content.kind === 'oversized'
                      ? 'This message exceeds the desktop display limit.'
                      : 'This message type is not supported yet.'}
                </p>
              </article>
            ))}
          </div>
          <form
            className="chat-composer"
            onSubmit={(e) => {
              e.preventDefault();
              void send();
            }}
          >
            <textarea
              aria-label="Message"
              value={draft}
              disabled={sending || submission.current !== null}
              placeholder="Write a message…"
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
            <div className="chat-row">
              <small>Enter to send · Shift+Enter for a new line</small>
              <Button
                variant="primary"
                type="submit"
                disabled={sending || (!draft.trim() && !submission.current)}
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
