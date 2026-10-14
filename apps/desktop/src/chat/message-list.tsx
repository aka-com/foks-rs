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
import { MessageTabOrderContext, MessageText } from './message-text';
import {
  RowTabStopContext,
  useMessageActions,
  type RowTabStop,
} from './message-actions';
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
 *
 * The rows take one tab stop, the current row (a roving tabindex): the row
 * focus last entered, else the first row on screen while the list is
 * scrolled away from its bottom, else the newest row. On a focused row, Up
 * and Down move to the previous and next row, Home and End to the first and
 * last row held, Enter opens the row's actions as a menu, and Esc calls
 * `onLeave`.
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
  onLeave,
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
  /**
   * Esc on a focused row: moves focus to the composer. Omitted where the
   * reader cannot send, which leaves Esc to `onKeyDown`.
   */
  onLeave?: () => void;
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
  // The first row on screen, which takes the tab stop while the list is
  // scrolled up, so Tab into the list does not scroll to the newest row.
  const [screenRow, setScreenRow] = useState<string | null>(null);
  const measureScreenRow = useCallback(() => {
    const element = scroller.current;
    setScreenRow(element ? rowOnScreen(element) : null);
  }, [scroller]);
  // Scrolling measures once per frame.
  const measuring = useRef<number | null>(null);
  const measureOnScroll = () => {
    if (measuring.current !== null) return;
    measuring.current = requestAnimationFrame(() => {
      measuring.current = null;
      measureNew();
      measureScreenRow();
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
  // One row is current, and it is the list's tab stop with its links and its
  // toolbar: the row focus last entered, while focus stays in the list and
  // the row is drawn; otherwise, while the list is away from its bottom, the
  // first row on screen, which focus reaches without scrolling the list;
  // otherwise the newest row. Other rows are reached with the arrow keys, by
  // pointer, or by focus entering the row, so a Tab across the list does not
  // stop at every message.
  const [focusedRow, setFocusedRow] = useState<string | null>(null);
  // The claim lasts while focus is in the list or in a menu or popover
  // (`.menu-portal`) a row opened; focus anywhere else, or on no element,
  // ends it. `focusin` also covers focus that moves on after the focused
  // element was removed without a `focusout`.
  useEffect(() => {
    const keeps = (node: EventTarget | null) =>
      node instanceof Element &&
      (Boolean(scroller.current?.contains(node)) ||
        node.closest('.menu-portal') !== null);
    const onFocusIn = (event: FocusEvent) => {
      if (!keeps(event.target)) setFocusedRow(null);
    };
    const onFocusOut = (event: FocusEvent) => {
      if (!keeps(event.relatedTarget)) setFocusedRow(null);
    };
    document.addEventListener('focusin', onFocusIn);
    document.addEventListener('focusout', onFocusOut);
    return () => {
      document.removeEventListener('focusin', onFocusIn);
      document.removeEventListener('focusout', onFocusOut);
    };
  }, [scroller]);
  const messageRows = [...held, ...unsent]
    .filter(
      (row) =>
        row.kind === 'message' ||
        row.kind === 'outgoing' ||
        row.kind === 'operation',
    )
    .map((row) => row.key);
  // Measured again when the rows drawn change, since rows added or removed
  // move the ones on screen.
  const rowKeys = messageRows.join(' ');
  useLayoutEffect(
    () => measureScreenRow(),
    [measureScreenRow, rowKeys, atBottom],
  );
  const currentRow =
    focusedRow !== null && messageRows.includes(focusedRow)
      ? focusedRow
      : !atBottom && screenRow !== null && messageRows.includes(screenRow)
        ? screenRow
        : (messageRows.at(-1) ?? null);
  const rowTabStop = useMemo<RowTabStop>(
    () => ({ current: currentRow, claim: setFocusedRow }),
    [currentRow],
  );
  // Keys on a focused row itself; keys in its links, toolbar and buttons are
  // theirs. A key at either end of the rows is left to the list, which
  // scrolls.
  const onRowKey = (event: KeyboardEvent<HTMLDivElement>) => {
    const row = event.target;
    if (
      event.defaultPrevented ||
      event.altKey ||
      event.ctrlKey ||
      event.metaKey ||
      event.shiftKey ||
      !(row instanceof HTMLElement) ||
      !row.matches('article.chat-message')
    )
      return;
    if (event.key === 'Escape') {
      if (!onLeave) return;
      event.preventDefault();
      onLeave();
      return;
    }
    if (!['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return;
    const rows = Array.from(
      event.currentTarget.querySelectorAll<HTMLElement>('article.chat-message'),
    );
    const at = rows.indexOf(row);
    const next =
      event.key === 'Home'
        ? rows[0]
        : event.key === 'End'
          ? rows.at(-1)
          : rows[at + (event.key === 'ArrowUp' ? -1 : 1)];
    if (!next || next === row) return;
    event.preventDefault();
    next.focus({ preventScroll: true });
    next.scrollIntoView?.({ block: 'nearest' });
  };
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
            rowKey={row.key}
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
        onKeyDown={onRowKey}
        // The current row is the list's tab stop; the list itself is one only
        // while it draws no row, and takes focus when a control in it goes.
        tabIndex={currentRow === null ? 0 : -1}
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
        <RowTabStopContext.Provider value={rowTabStop}>
          {held.map(draw)}
        </RowTabStopContext.Provider>
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
        <RowTabStopContext.Provider value={rowTabStop}>
          {unsent.map(draw)}
        </RowTabStopContext.Provider>
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
  const {
    open,
    onContextMenu,
    onKeyDown,
    onFocus,
    current,
    tabIndex,
    actions,
  } = useMessageActions(
    {
      rowKey: row.key,
      label: 'Actions for your message',
      text: op.text || undefined,
    },
    bridge,
  );
  return (
    <article
      className={open ? 'chat-message actions-open' : 'chat-message'}
      data-row={row.key}
      data-operation={op.id}
      tabIndex={tabIndex}
      onContextMenu={onContextMenu}
      onKeyDown={onKeyDown}
      onFocus={onFocus}
    >
      {op.text && (
        <MessageTabOrderContext.Provider value={current}>
          <MessageText text={op.text} actions={bridge} mentions={mentions} />
        </MessageTabOrderContext.Provider>
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

/**
 * The `data-row` key of the first row whose top edge is within the visible
 * area of `root`, else of the row that spans its top edge. Rows are in
 * document order, so the search halves the list. `null` when no row is laid
 * out on screen.
 */
function rowOnScreen(root: HTMLElement): string | null {
  const rows = root.querySelectorAll<HTMLElement>('article[data-row]');
  const { top, bottom } = root.getBoundingClientRect();
  let low = 0;
  let high = rows.length;
  while (low < high) {
    const middle = (low + high) >> 1;
    if (rows[middle].getBoundingClientRect().top >= top) high = middle;
    else low = middle + 1;
  }
  const below = rows[low]?.getBoundingClientRect();
  if (below && below.height > 0 && below.top < bottom)
    return rows[low].dataset.row ?? null;
  const spanning = rows[low - 1]?.getBoundingClientRect();
  if (spanning && spanning.height > 0 && spanning.bottom > top)
    return rows[low - 1].dataset.row ?? null;
  return null;
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
