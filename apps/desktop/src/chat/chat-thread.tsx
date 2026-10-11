import { MessageText } from './message-text';
import type { Bridge } from '../bridge';
import {
  Fragment,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import type { KeyboardEvent, ReactNode, Ref, RefObject } from 'react';
import { Button, Chip, Icon } from '../components';
import { ChatAlerts, failureAlert, type ChatAlert } from './chat-alerts';
import type { ChatAction, ChatChannel, ChatReply } from '../chat-contract';
import { plural, shortId } from '../model';
import { AccountMark } from '../screens/account-switcher';
import { TEXT_LIMIT_LABEL } from './use-chat-composer';
import { useMessageComposer } from './use-message-composer';
import { OutgoingRow } from './outgoing-row';
import { operationOf, phaseOf } from './send/outgoing';
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
  pendingRead = null,
  unread = null,
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
  /** The channel's read pointer as the server holds it. */
  readThrough: string | null;
  /**
   * A read this device staged and the server has not confirmed. The unread
   * count is measured from the further of the two, and the thread opens at
   * the first message after it when `unread` is above zero.
   */
  pendingRead?: string | null;
  /** The inbox's unread count for the channel, or `null` when it lists none. */
  unread?: string | null;
  markRead: (channel: string, sequence: string) => Promise<void>;
  acceptHistory: (
    page: Extract<import('../chat-contract').ChatResult, { kind: 'history' }>,
    before: string | null,
    placement?: import('./conversation-model').HistoryPlacement,
    sources?: readonly import('../chat-contract').ChatResult[],
  ) => import('./conversation-model').HistoryWindow | void;
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
      .map((m) => `${m.id}:${phaseOf(m)}:${m.text?.length ?? 0}`),
    ...pending.map((op) => `${op.id}:${op.state}:${op.text?.length ?? 0}`),
  ].join('|');
  // Read once: the thread opens at the first message that was unread when it
  // mounted, and later reads move the divider, not the open position.
  const readPoint =
    readThrough !== null &&
    pendingRead !== null &&
    BigInt(pendingRead) > BigInt(readThrough)
      ? pendingRead
      : readThrough;
  const [anchor] = useState(() =>
    readPoint !== null && unread !== null && BigInt(unread) > 0n
      ? readPoint
      : null,
  );
  const divider = useRef<HTMLDivElement>(null);
  const newerEdge = useRef<HTMLDivElement>(null);
  // The row an anchored open scrolls to: the NEW divider, else the first row
  // after the read pointer, which the divider omits when it is your own.
  const anchorRow = (): HTMLElement | null => {
    if (divider.current) return divider.current;
    const from = newFrom ?? anchor;
    if (from === null) return null;
    for (const row of scroller.current?.querySelectorAll<HTMLElement>(
      '[data-sequence]',
    ) ?? [])
      if (BigInt(row.dataset.sequence ?? '0') > BigInt(from)) return row;
    return null;
  };
  const { scroller, atBottom, onScroll, capture, release, reveal } =
    useChatViewport(
      history?.channel === channel.id ? history.messages : EMPTY_MESSAGES,
      pendingKey,
      anchorRow,
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
    detached,
    anchoring,
    load,
    loadNewer,
    loadLatest,
    reopen,
    retry,
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
    anchor,
  );
  const outgoingIds = new Set(
    outgoing.flatMap((m) => {
      const op = operationOf(m);
      return op ? [op.id] : [];
    }),
  );
  const composer = useComposerSize(draft, channel.readable && channel.writable);
  useEffect(() => {
    if (canSend && channel.readable && composer.current)
      autofocusComposer?.(composer.current);
  }, [autofocusComposer, canSend, channel.readable, composer]);
  // A placement waits for its page; a failed read brings none.
  useEffect(() => {
    if (failure) release();
  }, [failure, release]);
  const { newFrom, readError, markThrough, dismissNew } = useChatReadIntent(
    channel.id,
    messages,
    atBottom,
    readThrough,
    markRead,
    !anchoring,
    scroller,
    readPoint,
  );
  // Arrivals are counted against the newest row held when the reader left
  // the bottom of an attached window. A detached window counts none, since
  // it takes no incremental reads.
  const [arrivalBase, setArrivalBase] = useState<string | null>(null);
  useEffect(() => {
    if (atBottom || detached || anchoring) setArrivalBase(null);
    else setArrivalBase((base) => base ?? messages.at(-1)?.sequence ?? '0');
  }, [anchoring, atBottom, detached, messages]);
  // Rows are sorted, so the count reads only the arrivals at the end.
  let arrivals = 0;
  if (arrivalBase !== null)
    for (
      let index = messages.length - 1;
      index >= 0 && BigInt(messages[index].sequence) > BigInt(arrivalBase);
      index--
    )
      if (actor === null || messages[index].sender !== actor) arrivals++;
  const jumpToLatest = () => {
    // A detached window does not hold the newest row: read it.
    if (detached) {
      void loadLatest();
      return;
    }
    const element = scroller.current;
    if (!element) return;
    element.scrollTop = element.scrollHeight;
    onScroll();
  };
  // The banner stands in for the divider while the first new message is
  // below the list or older than the rows it holds.
  const unreadCount = unread === null ? 0 : Number(unread);
  const [newBelow, setNewBelow] = useState(false);
  const measureNew = useCallback(() => {
    const element = scroller.current;
    const row = divider.current;
    setNewBelow(
      Boolean(
        element &&
        row &&
        row.getBoundingClientRect().top >
          element.getBoundingClientRect().bottom,
      ),
    );
  }, [scroller]);
  useLayoutEffect(
    () => measureNew(),
    [measureNew, messages, newFrom, atBottom],
  );
  // Scrolling measures once per frame.
  const measuring = useRef<number | null>(null);
  const measureOnScroll = () => {
    if (measuring.current !== null) return;
    measuring.current = requestAnimationFrame(() => {
      measuring.current = null;
      measureNew();
    });
  };
  useEffect(
    () => () => {
      if (measuring.current !== null) cancelAnimationFrame(measuring.current);
    },
    [],
  );
  const newUnloaded =
    newFrom !== null &&
    before !== null &&
    messages.length > 0 &&
    BigInt(messages[0].sequence) > BigInt(newFrom) + 1n;
  const firstNew = useMemo(
    () =>
      newFrom === null || newUnloaded
        ? undefined
        : messages.find((m) => BigInt(m.sequence) > BigInt(newFrom)),
    [messages, newFrom, newUnloaded],
  );
  const showBanner =
    newFrom !== null && unreadCount > 0 && (newBelow || newUnloaded);
  // The banner, the newer edge and the jump button unmount once their action
  // completes. Focus inside one when it goes moves to the list rather than
  // to the document body. The ref cleanup runs before React removes the node.
  const returnFocus = useCallback(
    (node: HTMLElement | null) => () => {
      if (node?.contains(document.activeElement))
        scroller.current?.focus({ preventScroll: true });
    },
    [scroller],
  );
  const jumpToNew = () => {
    if (divider.current) reveal(divider.current);
    else if (newFrom !== null) void reopen(newFrom);
    scroller.current?.focus({ preventScroll: true });
  };
  // Marks through the newest message the inbox published, which a detached
  // window has not loaded, and removes the divider for the rest of the visit.
  // Without a published head, a detached window cannot state where the
  // unread messages end, so its divider stays.
  const markedAll = useRef<string | null>(null);
  const markAll = () => {
    const newest = position ?? messages.at(-1)?.sequence;
    if (newest !== undefined && BigInt(newest) > 0n) {
      markThrough(newest);
      markedAll.current = newest;
    }
    if (position !== null || !detached) dismissNew();
  };
  // A message sent from a detached window belongs after rows it has not
  // loaded, so sending reads the newest page first.
  const sendLatest = () => {
    if (send() && detached) void loadLatest();
  };
  const markOnEscape = (event: KeyboardEvent<HTMLElement>) => {
    if (
      event.key !== 'Escape' ||
      event.defaultPrevented ||
      event.altKey ||
      event.ctrlKey ||
      event.metaKey ||
      event.shiftKey ||
      // Ignore keys whose target is outside this element, such as portaled
      // menus and dialogs.
      !(event.target instanceof Node) ||
      !event.currentTarget.contains(event.target) ||
      draft !== '' ||
      // Nothing to mark or dismiss, or already marked through the same
      // message: leave Escape to the shell's handler.
      (unreadCount === 0 && !divider.current) ||
      markedAll.current === (position ?? messages.at(-1)?.sequence)
    )
      return;
    event.preventDefault();
    markAll();
  };
  const newerInView = useInView(newerEdge, scroller, detached, messages);
  useEffect(() => {
    if (newerInView && detached && !busy && !failure) void loadNewer();
  }, [busy, detached, failure, loadNewer, newerInView]);
  const newerCount =
    detached && position !== null && messages.length > 0
      ? Number(BigInt(position) - BigInt(messages.at(-1)!.sequence))
      : 0;
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
            { label: 'Retry', disabled: busy, run: () => void retry() },
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
          <div className="chat-messages-wrap" onKeyDown={markOnEscape}>
            {showBanner && (
              <div
                className="chat-new-banner"
                ref={returnFocus}
                role="group"
                aria-label="New messages"
              >
                <span>
                  <b>{plural(unreadCount, 'new message')}</b>
                  {firstNew && ` since ${sinceTime(firstNew.insert_time)}`}
                </span>
                <Button size="sm" onClick={jumpToNew}>
                  Jump to first new
                </Button>
                <Button size="sm" aria-keyshortcuts="Escape" onClick={markAll}>
                  Mark as read <kbd aria-hidden="true">Esc</kbd>
                </Button>
              </div>
            )}
            <div
              className="chat-messages"
              data-chat-channel={channel.id}
              data-chat-store={storeId}
              ref={scroller}
              onScroll={() => {
                onScroll();
                measureOnScroll();
              }}
              tabIndex={0}
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
                // The divider marks where the new messages begin, so it is
                // not drawn over a window that starts after that.
                const isNew =
                  newFrom !== null &&
                  !own &&
                  BigInt(m.sequence) > BigInt(newFrom) &&
                  (index === 0
                    ? !newUnloaded
                    : BigInt(messages[index - 1].sequence) <= BigInt(newFrom));
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
                        ref={divider}
                      >
                        <span>New</span>
                      </div>
                    )}
                    <article
                      className={
                        grouped ? 'chat-message grouped' : 'chat-message'
                      }
                      data-message={m.id}
                      data-sequence={m.sequence}
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
                              : m.content.kind === 'unverifiable'
                                ? 'This message could not be decrypted or verified.'
                                : 'This message type is not supported yet.'}
                          </p>
                        )}
                      </div>
                    </article>
                  </Fragment>
                );
              })}
              {detached && (
                <div
                  className="chat-history-edge chat-history-newer"
                  ref={newerEdge}
                >
                  {/* Marked busy rather than disabled while a page loads, so
                      focus stays on it until the page attaches the window. */}
                  <Button
                    size="sm"
                    busy={busy}
                    aria-disabled={busy || undefined}
                    ref={returnFocus}
                    onClick={busy ? undefined : () => void loadNewer()}
                  >
                    Load newer messages
                  </Button>
                  {newerCount > 0 && (
                    <small>{plural(newerCount, 'newer message')}</small>
                  )}
                </div>
              )}
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
            {(!atBottom || detached) && messages.length > 0 && (
              <Button
                size="sm"
                icon="chevronDown"
                className={arrivals ? 'chat-jump has-count' : 'chat-jump'}
                aria-label={
                  arrivals
                    ? `${plural(arrivals, 'new message')}, jump to latest`
                    : 'Jump to latest'
                }
                title="Jump to latest"
                ref={returnFocus}
                onClick={jumpToLatest}
              >
                {arrivals ? plural(arrivals, 'new message') : 'Jump to latest'}
              </Button>
            )}
          </div>
          {channel.writable ? (
            <form
              className="chat-composer"
              onSubmit={(e) => {
                e.preventDefault();
                sendLatest();
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
                  markOnEscape(e);
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
                    sendLatest();
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

/** When the first new message arrived: the clock today, else the date too. */
function sinceTime(milliseconds: string): string {
  return messageDayKey(milliseconds) === messageDayKey(String(Date.now()))
    ? messageClock(milliseconds)
    : messageTime(milliseconds);
}

/**
 * Whether `target` is within 120px of the list's visible area. Measured
 * afresh whenever `key` changes, since rows added above or below move it.
 * Always false without `IntersectionObserver`; the control it marks stays a
 * button.
 */
function useInView(
  target: RefObject<HTMLElement | null>,
  root: RefObject<HTMLElement | null>,
  mounted: boolean,
  key: unknown,
): boolean {
  // The `key` under which the target was last reported in view: a report
  // made before `key` changed is out of date, so it reads as not in view
  // until the observer reports again.
  const [inViewAt, setInViewAt] = useState<{ key: unknown } | null>(null);
  const keyRef = useRef(key);
  const observer = useRef<IntersectionObserver | null>(null);
  useEffect(() => {
    const element = target.current;
    if (
      !mounted ||
      !element ||
      !root.current ||
      typeof IntersectionObserver === 'undefined'
    )
      return;
    const current = new IntersectionObserver(
      (entries) =>
        setInViewAt(
          entries.some((entry) => entry.isIntersecting)
            ? { key: keyRef.current }
            : null,
        ),
      { root: root.current, rootMargin: '120px 0px' },
    );
    observer.current = current;
    current.observe(element);
    return () => {
      current.disconnect();
      observer.current = null;
      setInViewAt(null);
    };
  }, [mounted, root, target]);
  // Observing again yields a fresh first report under the new key.
  useEffect(() => {
    keyRef.current = key;
    const element = target.current;
    if (!observer.current || !element) return;
    observer.current.unobserve(element);
    observer.current.observe(element);
  }, [key, target]);
  const inView = inViewAt !== null && inViewAt.key === key;
  return inView;
}

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
