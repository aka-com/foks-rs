import { MessageText } from './message-text';
import type { Bridge } from '../bridge';
import { Fragment, useEffect, useLayoutEffect, useRef } from 'react';
import type { ReactNode, Ref } from 'react';
import { Button, Chip, Icon } from '../components';
import { ChatAlerts, failureAlert, type ChatAlert } from './chat-alerts';
import type { ChatAction, ChatChannel, ChatReply } from '../chat-contract';
import { plural, shortId } from '../model';
import { AccountMark } from '../screens/account-switcher';
import { TEXT_LIMIT_LABEL } from './use-chat-composer';
import { useMessageComposer } from './use-message-composer';
import { OutgoingRow } from './outgoing-row';
import { useChatReadIntent } from './use-chat-read-intent';
import { useChatViewport } from './use-chat-viewport';
import { useChatHistory } from './use-chat-history';
import { PendingRow } from './pending-row';
import {
  channelTitle,
  continuesMessageGroup,
  messageClock,
  messageDate,
  messageDay,
  messageDayKey,
  messageTime,
} from './presentation';
export function ChatThread({
  autofocusComposer,
  alerts = [],
  channel,
  teamName,
  memberCount,
  onInfo,
  infoOpen = false,
  infoRef,
  bridge,
  storeId,
  actor,
  scope,
  senderNames,
  request,
  refreshPending,
  revision,
  position = null,
  readThrough,
  markRead,
  acceptHistory,
  incrementalHistory = false,
  history,
  blockHistory,
  pending,
}: {
  autofocusComposer?: (element: HTMLTextAreaElement) => void;
  alerts?: readonly ChatAlert[];
  bridge: Bridge;
  storeId: string;
  history: import('./conversation-model').HistoryWindow | null;
  blockHistory: (channel: string) => void;
  channel: ChatChannel;
  /**
   * The team's name, for an alert that has to name the team: the header
   * itself names only the channel.
   */
  teamName?: string;
  /**
   * The channel's member count, from the roster the team page already loads.
   * `undefined` while that roster has not arrived for the open team — the
   * header omits the count rather than claiming zero.
   */
  memberCount?: number;
  /** Callback to toggle the channel info panel, which displays the member roster. */
  onInfo?: () => void;
  infoOpen?: boolean;
  /** The caller's handle on the ⓘ, so closing the panel can focus it again. */
  infoRef?: Ref<HTMLButtonElement>;
  pending: import('./operations').TrackedOperation[];
  actor: string | null;
  scope: import('../chat-contract').ChatScope | null;
  senderNames: Map<string, string>;
  request: (a: ChatAction) => Promise<ChatReply>;
  refreshPending: () => Promise<void>;
  revision: number;
  incrementalHistory?: boolean;
  /**
   * The newest sequence the inbox last published for this channel, or `null`
   * when it lists no conversation for it. Gates the incremental tail: a
   * revision that did not move this asks the agent for nothing.
   */
  position?: string | null;
  readThrough: string | null;
  markRead: (channel: string, sequence: string) => Promise<void>;
  acceptHistory: (
    page: Extract<import('../chat-contract').ChatResult, { kind: 'history' }>,
    before: string | null,
    replace?: boolean,
  ) => void;
}): ReactNode {
  const {
    draft,
    setDraft,
    sendError,
    draftBytes,
    send,
    overLimit,
    nearLimit,
    canSend,
    loadError,
    retryLoad,
    messages: outgoing,
    queueFull,
    cleanupError,
    service: sends,
  } = useMessageComposer(storeId, channel, scope);
  const pendingKey = [
    ...outgoing
      .filter((m) => !m.observed)
      .map((m) => `${m.id}:${m.phase}:${m.text?.length ?? 0}`),
    ...pending.map((op) => `${op.id}:${op.state}:${op.text?.length ?? 0}`),
  ].join('|');
  const { scroller, atBottom, onScroll, capture } = useChatViewport(
    history?.channel === channel.id ? history.messages : EMPTY_MESSAGES,
    pendingKey,
  );
  const {
    messages,
    before,
    missing,
    error,
    failure,
    busy,
    loaded,
    initialLoading,
    load,
  } = useChatHistory(
    channel,
    request,
    revision,
    acceptHistory,
    history,
    blockHistory,
    capture,
    incrementalHistory,
    position,
  );
  const outgoingIds = new Set(
    outgoing.flatMap((m) => (m.operation ? [m.operation.id] : [])),
  );
  const composer = useComposerSize(draft, channel.readable && channel.writable);
  useEffect(() => {
    if (canSend && channel.readable && composer.current)
      autofocusComposer?.(composer.current);
  }, [autofocusComposer, canSend, channel.readable, composer]);
  const { newFrom, readError } = useChatReadIntent(
    channel.id,
    messages,
    atBottom,
    readThrough,
    markRead,
    true,
  );
  const jumpToLatest = () => {
    const element = scroller.current;
    if (!element) return;
    element.scrollTop = element.scrollHeight;
    onScroll();
  };
  const title = channelTitle(channel);
  return (
    <>
      <div className="chat-thread-header">
        <h2>{title}</h2>
        {(channel.description?.trim() || memberCount !== undefined) && (
          <p className="chat-thread-sub">
            {channel.description?.trim() ||
              (memberCount === undefined ? '' : plural(memberCount, 'member'))}
          </p>
        )}
        {channel.admin && <Chip>Admins</Chip>}
        {!channel.readable && <Chip tone="warn">Restricted</Chip>}
        <span className="chat-thread-spacer" />
        {onInfo && (
          <Button
            variant="quiet"
            className="chat-head-act"
            icon="info"
            title="Channel info"
            aria-label="Channel info"
            on={infoOpen}
            ref={infoRef}
            onClick={onInfo}
          />
        )}
      </div>
      <ChatAlerts
        alerts={[
          ...alerts,
          failureAlert(failure, teamName, [
            { label: 'Retry', disabled: busy, run: () => void load() },
          ]),
          // A read mark the server refused for good: nothing to retry here,
          // the next mark clears it.
          { message: readError, severity: 'warn' },
          {
            message: missing
              ? 'Some earlier messages could not be checked. This history has incomplete verification.'
              : '',
            severity: 'warn',
          },
          { message: channel.writable ? sendError : '', severity: 'crit' },
          {
            message: channel.writable ? loadError : '',
            severity: 'crit',
            label: 'Saved messages could not be loaded.',
            actions: [{ label: 'Retry local recovery', run: retryLoad }],
          },
          {
            message: channel.writable ? cleanupError : '',
            severity: 'info',
            label: 'Local message cleanup is pending.',
          },
          {
            message:
              channel.writable && queueFull
                ? 'Too many messages are waiting to be sent in this channel. Your draft is kept until one of them goes.'
                : '',
            severity: 'warn',
          },
        ]}
      />
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
          <div className="chat-messages-wrap">
            <div
              className="chat-messages"
              data-chat-channel={channel.id}
              data-chat-store={storeId}
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
                  <small>Beginning of the channel</small>
                ) : null}
              </div>
              {initialLoading && (
                <p
                  className="chat-quiet chat-messages-empty"
                  role="status"
                  aria-label="Loading messages"
                >
                  Loading messages…
                </p>
              )}
              {loaded &&
                !messages.length &&
                !pending.length &&
                !busy &&
                !error && (
                  <div className="chat-empty chat-messages-empty">
                    <div>
                      <Icon name="chat" size={30} />
                      <b>No messages yet</b>
                      <span>
                        Send a message to start the conversation.{' '}
                        {memberCount !== undefined && teamName
                          ? `Only the ${plural(memberCount, 'member')} of ${teamName} can read it.`
                          : teamName
                            ? `Only the members of ${teamName} can read it.`
                            : ''}
                      </span>
                    </div>
                  </div>
                )}
              {messages.map((m, index) => {
                const own = actor !== null && m.sender === actor;
                const isNew =
                  newFrom !== null &&
                  !own &&
                  BigInt(m.sequence) > BigInt(newFrom) &&
                  (index === 0 ||
                    BigInt(messages[index - 1].sequence) <= BigInt(newFrom));
                const avatarName = m.sender
                  ? (senderNames.get(m.sender) ?? shortId(m.sender))
                  : 'Team member';
                const author = own ? 'You' : avatarName;
                // The day is carried once, above the first message of each
                // local day, so no row repeats it.
                const day = messageDayKey(m.insert_time);
                const newDay =
                  Boolean(day) &&
                  (index === 0 ||
                    messageDayKey(messages[index - 1].insert_time) !== day);
                const grouped =
                  !isNew &&
                  !newDay &&
                  continuesMessageGroup(messages[index - 1], m);
                return (
                  <Fragment key={sends.messageKey(storeId, m.id)}>
                    {newDay && (
                      <div className="chat-daysep" role="separator">
                        <span>{messageDay(m.insert_time)}</span>
                      </div>
                    )}
                    {isNew && (
                      <div
                        className="chat-divider"
                        role="separator"
                        aria-label="New messages"
                      >
                        <span>New</span>
                      </div>
                    )}
                    <article
                      className={
                        grouped ? 'chat-message grouped' : 'chat-message'
                      }
                      data-message={m.id}
                      title={grouped ? messageTime(m.insert_time) : undefined}
                    >
                      <AccountMark name={avatarName} className="chat-avatar" />
                      <div className="chat-message-body">
                        <header className={grouped ? 'offscreen' : undefined}>
                          <span
                            className={own ? 'chat-sender you' : 'chat-sender'}
                            title={m.sender ?? undefined}
                          >
                            {author}
                          </span>
                          <time
                            dateTime={messageDate(m.insert_time)?.toISOString()}
                            title={`Sent ${messageTime(m.send_time)} · Message #${m.sequence}`}
                          >
                            {messageClock(m.insert_time)}
                          </time>
                        </header>
                        {m.content.kind === 'text' ? (
                          <MessageText text={m.content.text} actions={bridge} />
                        ) : (
                          <p className="chat-unsupported">
                            {m.content.kind === 'oversized'
                              ? 'This message exceeds the desktop display limit.'
                              : 'This message type is not supported yet.'}
                          </p>
                        )}
                      </div>
                    </article>
                  </Fragment>
                );
              })}
              {outgoing
                .filter((m) => !m.observed)
                .map((message, index) => (
                  <OutgoingRow
                    key={message.id}
                    message={message}
                    grouped={index > 0}
                    avatarName={
                      actor
                        ? (senderNames.get(actor) ?? shortId(actor))
                        : 'Team member'
                    }
                    storeId={storeId}
                    service={sends}
                    bridge={bridge}
                  />
                ))}
              {pending
                .filter((op) => !outgoingIds.has(op.id))
                .map((op) => (
                  <article
                    className="chat-message"
                    key={op.id}
                    data-operation={op.id}
                  >
                    {op.text && <MessageText text={op.text} actions={bridge} />}
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
                variant="quiet"
                size="sm"
                icon="chevronDown"
                className="chat-jump"
                aria-label="Jump to latest"
                title="Jump to latest"
                onClick={jumpToLatest}
              />
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
              <textarea
                ref={composer}
                aria-label="Message"
                value={draft}
                placeholder={`Message ${title}`}
                rows={1}
                onChange={(e) => setDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (
                    e.key !== 'Enter' ||
                    e.nativeEvent.isComposing ||
                    e.keyCode === 229
                  )
                    return;
                  if (e.altKey) {
                    e.preventDefault();
                    const field = e.currentTarget;
                    field.setRangeText(
                      '\n',
                      field.selectionStart,
                      field.selectionEnd,
                      'end',
                    );
                    setDraft(field.value);
                  } else if (!e.shiftKey) {
                    e.preventDefault();
                    void send();
                  }
                }}
              />
              <button
                className="chat-send"
                type="submit"
                aria-label="Send"
                title="Send"
                disabled={!canSend || overLimit || !draft.trim()}
              >
                <Icon name="send" size={16} />
              </button>
              {nearLimit && (
                <small
                  className={overLimit ? 'chat-meter over' : 'chat-meter'}
                  aria-live="polite"
                >
                  {Math.ceil(draftBytes / 1024)} KiB of {TEXT_LIMIT_LABEL}
                </small>
              )}
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

const EMPTY_MESSAGES: import('../chat-contract').ChatMessage[] = [];

function resizeComposer(element: HTMLTextAreaElement): void {
  // Reset first so deleting text or clearing a sent draft shrinks the field.
  element.style.height = 'auto';
  const border = element.offsetHeight - element.clientHeight;
  element.style.height = `${element.scrollHeight + border}px`;
}

function useComposerSize(draft: string, visible: boolean) {
  const ref = useRef<HTMLTextAreaElement>(null);
  useLayoutEffect(() => {
    if (ref.current) resizeComposer(ref.current);
  }, [draft, visible]);
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element || typeof ResizeObserver === 'undefined') return;
    let width = element.clientWidth;
    const observer = new ResizeObserver(() => {
      // Reflow when the window or channel-info panel changes the available
      // width, without responding to our own height adjustments.
      if (element.clientWidth === width) return;
      width = element.clientWidth;
      resizeComposer(element);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [visible]);
  return ref;
}
