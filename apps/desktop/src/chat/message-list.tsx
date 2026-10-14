import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import type { KeyboardEvent, ReactNode, RefObject } from 'react';
import { Button, Icon } from '../components';
import type { ChatChannel } from '../chat-contract';
import { plural } from '../model';
import { useChatConversationContext } from './conversation-provider';
import { HistoryRow } from './message-row';
import { OutgoingRow } from './outgoing-row';
import { PendingRow } from './pending-row';
import { MessageText } from './message-text';
import { useMessageActions } from './message-actions';
import { messageClock, messageDayKey, messageTime } from './presentation';
import { splitThreadRows, type ThreadRow } from './thread-rows';
import type { useChatHistory } from './use-chat-history';
import type { useChatViewport } from './use-chat-viewport';

type History = Pick<
  ReturnType<typeof useChatHistory>,
  | 'messages'
  | 'before'
  | 'error'
  | 'failure'
  | 'busy'
  | 'loading'
  | 'loaded'
  | 'initialLoading'
  | 'detached'
  | 'anchoring'
  | 'load'
  | 'loadNewer'
  | 'loadLatest'
  | 'reopen'
>;
type Viewport = Pick<
  ReturnType<typeof useChatViewport>,
  'scroller' | 'atBottom' | 'onScroll' | 'reveal'
>;

/**
 * A readable channel's scrolling message list: the new-messages banner, the
 * history edges, the rows, and the jump to the latest message. The thread
 * owns the history window, the viewport and the read intent; this list owns
 * what it measures and draws from them.
 */
export function MessageList({
  channel,
  rows,
  history,
  viewport,
  divider,
  newFrom,
  newUnloaded,
  unread,
  position,
  hasOperations,
  teamName,
  memberCount,
  onMarkAll,
  onKeyDown,
  onQuote,
}: {
  channel: ChatChannel;
  /** The rows {@link threadRows} derived from the same window. */
  rows: readonly ThreadRow[];
  history: History;
  viewport: Viewport;
  /** The NEW divider, which the thread anchors an open on and marks by. */
  divider: RefObject<HTMLDivElement | null>;
  newFrom: string | null;
  newUnloaded: boolean;
  /** The inbox's unread count for the channel. */
  unread: number;
  /** The newest sequence the inbox published, or `null` for none. */
  position: string | null;
  /** Whether the channel has tracked operations, which rule out "No messages". */
  hasOperations: boolean;
  teamName?: string;
  memberCount?: number;
  onMarkAll: () => void;
  onKeyDown: (event: KeyboardEvent<HTMLElement>) => void;
  /** Writes a quote into the composer; omitted where the reader cannot send. */
  onQuote?: (quote: string) => void;
}): ReactNode {
  const { actor, storeId, refreshPending } = useChatConversationContext();
  const {
    messages,
    before,
    error,
    failure,
    busy,
    loading,
    loaded,
    initialLoading,
    detached,
    anchoring,
    load,
    loadNewer,
    loadLatest,
    reopen,
  } = history;
  const { scroller, atBottom, onScroll, reveal } = viewport;
  const newerEdge = useRef<HTMLDivElement>(null);
  const olderEdge = useRef<HTMLDivElement>(null);
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
  }, [divider, scroller]);
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
  const firstNew = useMemo(
    () =>
      newFrom === null || newUnloaded
        ? undefined
        : messages.find((m) => BigInt(m.sequence) > BigInt(newFrom)),
    [messages, newFrom, newUnloaded],
  );
  const showBanner =
    newFrom !== null && unread > 0 && (newBelow || newUnloaded);
  // The banner, the history edges' buttons and the jump button unmount once
  // their action completes. Focus inside one when it goes moves to the list
  // rather than to the document body. The ref cleanup runs before React
  // removes the node.
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
  // Older pages load as the top of the list comes near the viewport. The
  // button stays for the keyboard, and for a webview without
  // IntersectionObserver, where it is the only way to page back.
  const olderInView = useInView(olderEdge, scroller, before !== null, messages);
  useEffect(() => {
    if (olderInView && before !== null && !anchoring && !busy && !failure)
      void load(before);
  }, [anchoring, before, busy, failure, load, olderInView]);
  const newerInView = useInView(newerEdge, scroller, detached, messages);
  useEffect(() => {
    if (newerInView && detached && !busy && !failure) void loadNewer();
  }, [busy, detached, failure, loadNewer, newerInView]);
  const newerCount =
    detached && position !== null && messages.length > 0
      ? Number(BigInt(position) - BigInt(messages.at(-1)!.sequence))
      : 0;
  // The rows this device has not seen in history follow the newer edge.
  const { held, unsent } = splitThreadRows(rows);
  const draw = (row: ThreadRow): ReactNode => {
    switch (row.kind) {
      case 'day':
        return (
          <div className="chat-daysep" role="separator" key={row.key}>
            <span>{row.label}</span>
          </div>
        );
      case 'new':
        return (
          <div
            className="chat-divider"
            role="separator"
            aria-label="New messages"
            ref={divider}
            key={row.key}
          >
            <span>New</span>
          </div>
        );
      case 'message':
        return <HistoryRow row={row} key={row.key} onQuote={onQuote} />;
      case 'outgoing':
        return (
          <OutgoingRow
            key={row.key}
            message={row.message}
            grouped={row.grouped}
            avatarName={row.avatarName}
          />
        );
      case 'operation':
        return (
          <OperationRow
            key={row.key}
            row={row}
            onChange={() => {
              void refreshPending();
              void load();
            }}
          />
        );
    }
  };
  return (
    <div className="chat-messages-wrap" onKeyDown={onKeyDown}>
      {showBanner && (
        <div
          className="chat-new-banner"
          ref={returnFocus}
          role="group"
          aria-label="New messages"
        >
          <span>
            <b>{plural(unread, 'new message')}</b>
            {firstNew && ` since ${sinceTime(firstNew.insert_time)}`}
          </span>
          <Button size="sm" onClick={jumpToNew}>
            Jump to first new
          </Button>
          <Button size="sm" aria-keyshortcuts="Escape" onClick={onMarkAll}>
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
        <div className="chat-history-edge chat-history-older" ref={olderEdge}>
          {before ? (
            <>
              {loading === 'older' && (
                <small role="status">Loading older messages…</small>
              )}
              <Button
                size="sm"
                className={
                  typeof IntersectionObserver === 'undefined'
                    ? undefined
                    : 'chat-history-fallback'
                }
                busy={loading === 'older'}
                aria-disabled={busy || undefined}
                ref={returnFocus}
                onClick={busy ? undefined : () => void load(before)}
              >
                Load older messages
              </Button>
            </>
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
        {loaded && !messages.length && !hasOperations && !busy && !error && (
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
        {held.map(draw)}
        {detached && (
          <div className="chat-history-edge chat-history-newer" ref={newerEdge}>
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
        {unsent.map(draw)}
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
  );
}

/** A tracked operation that no outgoing message of this device carries. */
function OperationRow({
  row,
  onChange,
}: {
  row: Extract<ThreadRow, { kind: 'operation' }>;
  onChange: () => void;
}): ReactNode {
  const { bridge, storeId, mentions } = useChatConversationContext();
  const op = row.operation;
  const { open, onContextMenu, onKeyDown, actions } = useMessageActions(
    { text: op.text || undefined },
    bridge,
  );
  return (
    <article
      className={open ? 'chat-message actions-open' : 'chat-message'}
      data-operation={op.id}
      onContextMenu={onContextMenu}
      onKeyDown={onKeyDown}
    >
      {op.text && (
        <MessageText text={op.text} actions={bridge} mentions={mentions} />
      )}
      <PendingRow
        storeId={storeId}
        operation={op}
        channelName={undefined}
        onChange={onChange}
      />
      {actions}
    </article>
  );
}

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
