import { MessageText } from './message-text';
import type { Bridge } from '../bridge';
import { Fragment, useId } from 'react';
import type { ReactNode, Ref } from 'react';
import { Band, Button, Chip, Icon } from '../components';
import type { ChatAction, ChatChannel, ChatReply } from '../chat-contract';
import { plural, shortId } from '../model';
import { failure } from './actions';
import { TEXT_LIMIT_LABEL } from './use-chat-composer';
import { useMessageComposer } from './use-message-composer';
import { OutgoingRow } from './outgoing-row';
import { useChatReadIntent } from './use-chat-read-intent';
import { useChatViewport } from './use-chat-viewport';
import { useChatHistory } from './use-chat-history';
import { PendingRow } from './pending-row';
import {
  channelTitle,
  messageDate,
  messageTime,
  relativeMessageTime,
} from './presentation';
export function ChatThread({
  channel,
  teamName,
  memberCount,
  onFiles,
  onSearch,
  onSettings,
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
  refreshInbox,
  revision,
  readThrough,
  markRead,
  acceptHistory,
  history,
  blockHistory,
  pending,
}: {
  bridge: Bridge;
  storeId: string;
  history: import('./conversation-model').HistoryWindow | null;
  blockHistory: (channel: string) => void;
  channel: ChatChannel;
  /** The team the conversation header names before the channel. */
  teamName?: string;
  /**
   * The channel's member count, from the roster the team page already loads.
   * `undefined` while that roster has not arrived for the open team — the
   * header omits the count rather than claiming zero.
   */
  memberCount?: number;
  /** Opens the team's files; the header's folder button. */
  onFiles?: () => void;
  /** Moves to the inbox column's search field; the header's search button. */
  onSearch?: () => void;
  /** Opens the team's settings; the header's gear. */
  onSettings?: () => void;
  /** Opens the channel info panel; the header's ⓘ button. */
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
  /** Re-lists the team's channels and saved work; the header's Refresh. */
  refreshInbox: () => Promise<void>;
  revision: number;
  readThrough: string | null;
  markRead: (channel: string, sequence: string) => Promise<void>;
  acceptHistory: (
    page: Extract<import('../chat-contract').ChatResult, { kind: 'history' }>,
    before: string | null,
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
    setError,
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
  );
  const outgoingIds = new Set(
    outgoing.flatMap((m) => (m.operation ? [m.operation.id] : [])),
  );
  const hintId = useId();
  const waitingForSavedWork = !canSend && Boolean(draft.trim());
  const newFrom = useChatReadIntent(
    channel.id,
    messages,
    atBottom,
    readThrough,
    markRead,
    setError,
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
        <div className="chat-thread-title">
          <h2>
            {teamName && (
              <>
                <span className="team">{teamName}</span>
                {/* The separator is markup, not a CSS pseudo-element, so the
                    accessible name is "Team · #channel". */}
                <span className="sep" aria-hidden="true">
                  {' · '}
                </span>
              </>
            )}
            <span className="chan">{title}</span>
          </h2>
          {/* Who is here, and nothing else: the channel's description and its
              access line are both in the info panel, which draws the
              description under "Description" and the access under "Who can
              take part". A channel whose roster has not arrived has no count,
              and the line is empty rather than shortened. */}
          <p>
            {memberCount !== undefined && (
              <span className="chat-member-count">
                {plural(memberCount, 'member')}
              </span>
            )}
          </p>
        </div>
        {channel.admin && <Chip>Admins</Chip>}
        {!channel.readable && <Chip tone="warn">Restricted</Chip>}
        <Button
          variant="quiet"
          icon="again"
          aria-label="Refresh messages"
          title="Refresh this conversation and the channel list"
          disabled={busy}
          onClick={() => {
            void load();
            // One refresh: this history, the team's channels, and the saved
            // work the conversation is recovering.
            void refreshInbox().catch((e) => setError(failure(e)));
          }}
        />
        {onSearch && (
          <Button
            variant="quiet"
            icon="search"
            aria-label="Search teams and channels"
            // Chat has no message index, so the header's search is the
            // column's: it finds a team or a channel, never a message.
            title="Search teams and channels"
            onClick={onSearch}
          />
        )}
        {onFiles && (
          <Button
            variant="quiet"
            icon="folder"
            aria-label="Team files"
            title="Team files"
            onClick={onFiles}
          />
        )}
        {onSettings && (
          <Button
            variant="quiet"
            icon="gear"
            aria-label={`Team settings for ${teamName ?? 'this team'}`}
            title="Team settings"
            onClick={onSettings}
          />
        )}
        {onInfo && (
          <Button
            variant="quiet"
            icon="info"
            aria-label="Channel info"
            title="Channel info"
            on={infoOpen}
            ref={infoRef}
            onClick={onInfo}
          />
        )}
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
                  <small>Beginning of the conversation</small>
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
                  <Fragment key={sends.messageKey(storeId, m.id)}>
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
                          {relativeMessageTime(m.insert_time)}
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
                    </article>
                  </Fragment>
                );
              })}
              {outgoing
                .filter((m) => !m.observed)
                .map((message) => (
                  <OutgoingRow
                    key={message.id}
                    message={message}
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
              {loadError && (
                <Band severity="crit">
                  <span role="alert">
                    Saved messages could not be loaded. {loadError}
                  </span>
                  <Button size="sm" onClick={retryLoad}>
                    Retry local recovery
                  </Button>
                </Band>
              )}
              {cleanupError && (
                <Band severity="info">
                  <span role="status">
                    Local message cleanup is pending. {cleanupError}
                  </span>
                </Band>
              )}
              <textarea
                aria-label="Message"
                aria-describedby={waitingForSavedWork ? hintId : undefined}
                value={draft}
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
                {waitingForSavedWork ? (
                  <small id={hintId}>
                    Waiting for saved work. You can keep typing.
                  </small>
                ) : null}
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
                  disabled={!canSend || overLimit || !draft.trim()}
                >
                  Send
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

const EMPTY_MESSAGES: import('../chat-contract').ChatMessage[] = [];
