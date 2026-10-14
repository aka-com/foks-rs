import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import type { KeyboardEvent, ReactNode, Ref } from 'react';
import { Icon } from '../components';
import { ChatAlerts, failureAlert, type ChatAlert } from './chat-alerts';
import type { ChatChannel, ChatMessage } from '../chat-contract';
import { CHAT_PAGE_ROWS } from '../chat-limits';
import type { HistoryWindow } from './conversation-model';
import type { TrackedOperation } from './operations';
import { useChatConversationContext } from './conversation-provider';
import { useChatComposer } from './use-chat-composer';
import { phaseOf } from './send/outgoing';
import { useChatReadIntent } from './use-chat-read-intent';
import { useChatViewport } from './use-chat-viewport';
import { useChatHistory } from './use-chat-history';
import { channelTitle } from './presentation';
import { newMessagesUnloaded, threadRows } from './thread-rows';
import { ThreadHeader } from './thread-header';
import { MessageList } from './message-list';
import { MessageFindContext } from './message-text';
import { Composer } from './thread-composer';
import { quoteInsertion } from './message-actions';
import { channelRoster } from './mentions';
import { channelReadRole, type ChannelAudience } from './audience';
import { anyDialogOpen } from '/kit/overlay-primitives';
import { applePlatform } from './composer-format';
import { receiveChatFind, type FindReference } from './find';
import { FindBar } from './find-bar';
import { useChatFind } from './use-chat-find';

/**
 * One channel of the open team. The thread owns the channel's history
 * window, viewport, read intent and send state, and draws them through
 * {@link ThreadHeader}, {@link MessageList} and {@link Composer}. The
 * composer reads the draft itself, so typing does not re-render the thread.
 * What every channel of the team shares comes from the conversation provider.
 */
export function ChatThread({
  autofocusComposer,
  alerts = [],
  attention,
  channel,
  teamName,
  audience,
  onInfo,
  infoOpen = false,
  infoRef,
  revision,
  position = null,
  readThrough,
  pendingRead = null,
  unread = null,
  incrementalHistory = false,
  history,
  pending,
}: {
  autofocusComposer?: (element: HTMLTextAreaElement) => void;
  alerts?: readonly ChatAlert[];
  /** The team's unsent-work chip, which the header draws. */
  attention?: ReactNode;
  history: HistoryWindow | null;
  channel: ChatChannel;
  /**
   * The team's name, for an alert that has to name the team: the header
   * itself names only the channel.
   */
  teamName?: string;
  /**
   * Who can read and post in the channel, from the roster the team page
   * already loads. `undefined` while that roster has not arrived for the open
   * team — the header omits the count rather than claiming zero.
   */
  audience?: ChannelAudience;
  /** Callback to toggle the channel info panel, which lists the readers. */
  onInfo?: () => void;
  infoOpen?: boolean;
  /** The caller's handle on the ⓘ, so closing the panel can focus it again. */
  infoRef?: Ref<HTMLButtonElement>;
  pending: TrackedOperation[];
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
}): ReactNode {
  const {
    storeId,
    actor,
    scope,
    senderNames,
    request,
    markRead,
    acceptHistory,
    blockHistory,
    listPosition,
    keepListPosition,
    members,
  } = useChatConversationContext();
  const {
    setDraft,
    sendError,
    send,
    canRestore,
    restoreLatest,
    canSend,
    loadError,
    retryLoad,
    messages: outgoing,
    queueFull,
    cleanupError,
    service: sends,
  } = useChatComposer(storeId, channel, scope);
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
  // Also read once: the held window and where the reader left it. The thread
  // resumes there when the window still holds that row and, with unread
  // messages, also holds the first of them, which an anchored open would
  // otherwise read. A window that is not detached also reads the newest page
  // on open, which replaces it unless that page continues its tail: it
  // resumes only while the published head is at most a page past the tail.
  const [resume] = useState(() => {
    const saved = listPosition(channel.id);
    const window = history?.channel === channel.id ? history : null;
    const held = window?.messages ?? [];
    if (!saved || !held.some((m) => m.id === saved.message)) return null;
    const first = held[0];
    const last = held.at(-1);
    if (
      !window?.detached &&
      (position === null ||
        !last ||
        BigInt(position) - BigInt(last.sequence) > BigInt(CHAT_PAGE_ROWS))
    )
      return null;
    if (readPoint === null || unread === null || BigInt(unread) === 0n)
      return saved;
    return first &&
      last &&
      BigInt(first.sequence) <= BigInt(readPoint) + 1n &&
      BigInt(last.sequence) > BigInt(readPoint)
      ? saved
      : null;
  });
  const [anchor] = useState(() =>
    resume === null &&
    readPoint !== null &&
    unread !== null &&
    BigInt(unread) > 0n
      ? readPoint
      : null,
  );
  const divider = useRef<HTMLDivElement>(null);
  // A message find opened the channel around, until its page is drawn.
  const findTarget = useRef<string | null>(null);
  // The row an anchored open scrolls to: the message find opened the channel
  // around, else the NEW divider, else the first row after the read pointer,
  // which the divider omits when it is your own.
  const anchorRow = (): HTMLElement | null => {
    if (findTarget.current !== null) {
      const row = messageRow(viewport.scroller.current, findTarget.current);
      if (row) return row;
    }
    if (divider.current) return divider.current;
    const from = newFrom ?? anchor;
    if (from === null) return null;
    for (const row of viewport.scroller.current?.querySelectorAll<HTMLElement>(
      '[data-sequence]',
    ) ?? [])
      if (BigInt(row.dataset.sequence ?? '0') > BigInt(from)) return row;
    return null;
  };
  const viewport = useChatViewport(
    history?.channel === channel.id ? history.messages : EMPTY_MESSAGES,
    pendingKey,
    anchorRow,
    resume,
  );
  const { scroller, atBottom, capture, release, position: leaving } = viewport;
  // The list's position goes back to the history cache when the thread
  // unmounts, so the channel reopens where the reader left it.
  useLayoutEffect(
    () => () => keepListPosition(channel.id, leaving()),
    [channel.id, keepListPosition, leaving],
  );
  const thread = useChatHistory({
    channel,
    request,
    revision,
    history,
    position,
    anchor,
    resume: resume !== null,
    incremental: incrementalHistory,
    onAccepted: acceptHistory,
    onFatal: blockHistory,
    onLoading: capture,
  });
  const {
    messages,
    before,
    missing,
    verification,
    failure,
    busy,
    detached,
    anchoring,
  } = thread;
  // A placement waits for its page; a failed read brings none.
  useEffect(() => {
    if (!failure) return;
    release();
    findTarget.current = null;
  }, [failure, release]);
  const find = useChatFind({
    channel: channel.id,
    scroller,
    messages,
    before,
    request,
    onScroll: viewport.onScroll,
  });
  const { show: showFind, selectRow } = find;
  // ⌘F (Ctrl+F elsewhere) opens the find bar from anywhere in the window, as
  // does typing in the window header's field while its badge names this
  // channel. The shortcut sits on the document, so it tests for a dialog
  // itself.
  useEffect(() => {
    if (!channel.readable) return;
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (
        !(applePlatform() ? event.metaKey : event.ctrlKey) ||
        event.altKey ||
        event.shiftKey ||
        event.key.toLowerCase() !== 'f' ||
        event.defaultPrevented ||
        anyDialogOpen() ||
        scroller.current?.closest('[inert]')
      )
        return;
      event.preventDefault();
      showFind();
    };
    document.addEventListener('keydown', onKeyDown);
    const stop = receiveChatFind(showFind);
    return () => {
      document.removeEventListener('keydown', onKeyDown);
      stop();
    };
  }, [channel.readable, scroller, showFind]);
  // The message find opened is marked for two seconds and takes focus. One
  // the window holds is revealed in place; any other opens the channel
  // around it with the anchored read, which replaces the window rather than
  // growing it.
  const marked = useRef<{ row: HTMLElement; timer: number } | null>(null);
  const markFound = useCallback((row: HTMLElement) => {
    const last = marked.current;
    if (last) {
      clearTimeout(last.timer);
      last.row.removeAttribute('data-find-target');
    }
    row.setAttribute('data-find-target', '');
    row.focus({ preventScroll: true });
    marked.current = {
      row,
      timer: window.setTimeout(() => {
        row.removeAttribute('data-find-target');
        if (marked.current?.row === row) marked.current = null;
      }, FOUND_MARK_MS),
    };
  }, []);
  useEffect(
    () => () => {
      if (marked.current) clearTimeout(marked.current.timer);
    },
    [],
  );
  const openFound = (reference: FindReference) => {
    selectRow(reference.id);
    const row = messageRow(scroller.current, reference.id);
    if (row) {
      viewport.reveal(row);
      markFound(row);
      return;
    }
    findTarget.current = reference.id;
    void thread.reopen(String(BigInt(reference.sequence) - 1n));
  };
  useLayoutEffect(() => {
    const id = findTarget.current;
    if (id === null) return;
    const row = messageRow(scroller.current, id);
    if (row) {
      findTarget.current = null;
      markFound(row);
    } else if (!anchoring) findTarget.current = null;
  }, [anchoring, markFound, messages, scroller]);
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
  const unreadCount = unread === null ? 0 : Number(unread);
  const newUnloaded = newMessagesUnloaded(messages, newFrom, before !== null);
  const rows = threadRows(messages, outgoing, pending, {
    actor,
    newFrom,
    newUnloaded,
    names: senderNames,
    verification,
    keyOf: (m) => sends.messageKey(storeId, m.id),
  });
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
  // `@name` completes to the members who can read the channel, by the read
  // rule the info panel's reader list follows.
  const roster = useMemo(
    () => channelRoster(members, channelReadRole(channel)),
    [members, channel],
  );
  // Quote reply writes at the composer's caret, or where the caret was when
  // the field lost focus, and leaves the caret after the quote.
  const composerField = useRef<HTMLTextAreaElement>(null);
  const quote = (text: string) => {
    const field = composerField.current;
    if (!field) return;
    const at = field.selectionStart;
    field.setRangeText(
      quoteInsertion(field.value.slice(0, at), text),
      at,
      at,
      'end',
    );
    setDraft(field.value);
    field.focus();
  };
  // A message sent from a detached window belongs after rows it has not
  // loaded, so sending reads the newest page first.
  const sendLatest = () => {
    if (send() && detached) void thread.loadLatest();
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
      sends.draft(storeId, channel.id) !== '' ||
      // Nothing to mark or dismiss, or already marked through the same
      // message: leave Escape to the shell's handler.
      (unreadCount === 0 && !divider.current) ||
      markedAll.current === (position ?? messages.at(-1)?.sequence)
    )
      return;
    event.preventDefault();
    markAll();
  };
  return (
    <>
      <ThreadHeader
        channel={channel}
        audience={audience}
        onInfo={onInfo}
        infoOpen={infoOpen}
        infoRef={infoRef}
        attention={attention}
        onFind={
          channel.readable
            ? () => (find.open ? find.close() : showFind())
            : undefined
        }
        findOpen={find.open}
      />
      {channel.readable && find.open && (
        <FindBar
          find={find}
          title={channelTitle(channel)}
          oldest={messages[0]?.insert_time ?? null}
          olderAvailable={before !== null}
          actor={actor}
          names={senderNames}
          onOpen={openFound}
        />
      )}
      <ChatAlerts
        alerts={[
          ...alerts,
          failureAlert(failure, teamName, [
            { label: 'Retry', disabled: busy, run: () => void thread.retry() },
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
          <MessageFindContext.Provider value={find.holdsQuery}>
            <MessageList
              channel={channel}
              rows={rows}
              history={thread}
              viewport={viewport}
              divider={divider}
              newFrom={newFrom}
              newUnloaded={newUnloaded}
              unread={unreadCount}
              position={position}
              hasOperations={pending.length > 0}
              teamName={teamName}
              audience={audience}
              onMarkAll={markAll}
              onKeyDown={markOnEscape}
              onQuote={channel.writable ? quote : undefined}
              onLeave={
                channel.writable
                  ? () => composerField.current?.focus()
                  : undefined
              }
            />
          </MessageFindContext.Provider>
          {channel.writable ? (
            <Composer
              storeId={storeId}
              channel={channel.id}
              channelTitle={channelTitle(channel)}
              placeholder={`Message ${channelTitle(channel)}`}
              roster={roster}
              setDraft={setDraft}
              canSend={canSend}
              onSend={sendLatest}
              canRestore={canRestore}
              onRestore={restoreLatest}
              onKeyDown={markOnEscape}
              autofocus={autofocusComposer}
              fieldRef={composerField}
            />
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

const EMPTY_MESSAGES: ChatMessage[] = [];

/** How long a message find opened stays marked. */
const FOUND_MARK_MS = 2000;

/** The history row of message `id`, compared by attribute, not by selector. */
function messageRow(root: HTMLElement | null, id: string): HTMLElement | null {
  for (const row of root?.querySelectorAll<HTMLElement>('[data-message]') ?? [])
    if (row.dataset.message === id) return row;
  return null;
}
