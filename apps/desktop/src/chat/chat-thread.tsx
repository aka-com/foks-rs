import { Fragment, useEffect, useId, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { Band, Button, Chip, Icon } from '../components';
import type { ChatAction, ChatChannel, ChatReply } from '../chat-contract';
import { CHAT_TEXT_BYTES } from '../chat-contract';
import { shortId } from '../model';
import { failure, preparationCanChange, submissionId } from './actions';
import { useChatHistory } from './use-chat-history';
import { PendingRow } from './pending-row';
import {
  channelTitle,
  accessSummary,
  messageDate,
  messageTime,
} from './presentation';
const TEXT_LIMIT_LABEL = `${CHAT_TEXT_BYTES / 1024} KiB`;
export function ChatThread({
  channel,
  actor,
  senderNames,
  request,
  refreshPending,
  revision,
  readThrough,
  markRead,
  acceptHistory,
  history,
  blockHistory,
  pending,
}: {
  history: import('./conversation-model').HistoryWindow | null;
  blockHistory: (channel: string) => void;
  channel: ChatChannel;
  pending: import('./operations').TrackedOperation[];
  actor: string | null;
  senderNames: Map<string, string>;
  request: (a: ChatAction) => Promise<ChatReply>;
  refreshPending: () => Promise<void>;
  revision: number;
  readThrough: string | null;
  markRead: (channel: string, sequence: string) => Promise<void>;
  acceptHistory: (
    page: Extract<import('../chat-contract').ChatResult, { kind: 'history' }>,
    before: string | null,
  ) => void;
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
    atBottom,
    onScroll,
  } = useChatHistory(
    channel,
    request,
    revision,
    acceptHistory,
    history,
    blockHistory,
  );
  const active = useRef(false);
  useEffect(() => {
    active.current = true;
    return () => {
      active.current = false;
    };
  }, []);
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
    if (
      !channel.writable ||
      sendGuard.current ||
      (!submission.current && !draft.trim())
    )
      return;
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
          {channel.description && <p>{channel.description}</p>}
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
              {!messages.length && !pending.length && !busy && !error && (
                <p className="chat-quiet chat-messages-empty">
                  No messages yet. Send a message to start the conversation.
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
                              ? (senderNames.get(m.sender) ?? shortId(m.sender))
                              : 'Team member'}
                        </span>
                        <time
                          dateTime={messageDate(m.insert_time)?.toISOString()}
                          title={`Sent ${messageTime(m.send_time)} · inserted as message ${m.sequence}`}
                        >
                          {messageTime(m.insert_time)}
                        </time>
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
              {pending.map((op) => (
                <article
                  className="chat-message"
                  key={op.id}
                  data-operation={op.id}
                >
                  {op.text && <p>{op.text}</p>}
                  <PendingRow
                    operation={op}
                    channelName={undefined}
                    request={request}
                    onChange={() => {
                      void refreshPending();
                      void load();
                    }}
                  />
                </article>
              ))}
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
          {channel.writable ? (
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
                    sending ||
                    overLimit ||
                    (!draft.trim() && !submission.current)
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
          ) : (
            <p className="chat-quiet">
              Your current role can read this channel but cannot send messages.
            </p>
          )}
        </>
      )}
    </>
  );
}
