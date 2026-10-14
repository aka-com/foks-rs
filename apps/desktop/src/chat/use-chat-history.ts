import { normalizeCommandError } from '../bridge';
import type { CommandError } from '../bridge';
import { useCallback, useEffect, useRef, useState } from 'react';
import type {
  ChatAction,
  ChatChannel,
  ChatMessage,
  ChatReply,
  ChatResult,
} from '../chat-contract';
import type { AcceptHistory, HistoryWindow } from './conversation-model';
import { CHAT_PAGE_ROWS } from '../chat-limits';
import { diagnosticLog, hashId, outcomeForCode } from '../diagnostics/log';

const EMPTY_MESSAGES: ChatMessage[] = [];

/**
 * What a history read is for. `tail` reads the newest page and `incremental`
 * the rows after the held tail. `older` and `newer` page the held window at
 * either end. `anchor` opens the window at the first message after the read
 * pointer it is given, and `latest` replaces whatever is held with the newest
 * page.
 */
export type HistoryLoad =
  'tail' | 'incremental' | 'older' | 'newer' | 'anchor' | 'latest';

/** Rows above the first unread message that an anchored open also loads. */
export const ANCHOR_READ_ROWS = 10;

/**
 * The `before` cursor of the page that opens a channel read through
 * `readThrough`: the last {@link ANCHOR_READ_ROWS} read rows, then the
 * unread rows a page has room for. The agent reads the range that ends one
 * below the cursor, and a range past the head returns the rows that exist.
 */
export function anchorBefore(readThrough: string): string {
  return String(
    BigInt(readThrough) + 1n + BigInt(CHAT_PAGE_ROWS - ANCHOR_READ_ROWS),
  );
}

interface HistoryRead {
  load: HistoryLoad;
  cursor: string | null;
}

function lowest(messages: readonly ChatMessage[]): bigint | null {
  return messages.reduce<bigint | null>(
    (min, m) =>
      min === null || BigInt(m.sequence) < min ? BigInt(m.sequence) : min,
    null,
  );
}

function newest(messages: readonly ChatMessage[], floor = 0n): bigint {
  return messages.reduce(
    (max, m) => (BigInt(m.sequence) > max ? BigInt(m.sequence) : max),
    floor,
  );
}

/**
 * The newest end of the held window, which the next incremental or forward
 * read continues from.
 */
interface HeldEnd {
  channel: string;
  /** The newest held sequence, or `undefined` when no row is held. */
  tail: string | undefined;
  detached: boolean;
}

function endOf(window: HistoryWindow | null): HeldEnd | null {
  return (
    window && {
      channel: window.channel,
      tail: window.messages.at(-1)?.sequence,
      detached: window.detached,
    }
  );
}

/**
 * The newest sequence among `messages` and `floor`, as the row states it.
 * Unlike {@link newest}, it keeps `undefined` (no row held) apart from
 * sequence 0.
 */
function tailOf(
  messages: readonly ChatMessage[],
  floor: string | undefined,
): string | undefined {
  return messages.reduce(
    (max, m) =>
      max === undefined || BigInt(m.sequence) > BigInt(max) ? m.sequence : max,
    floor,
  );
}

/** The channel {@link useChatHistory} reads, and what it reports to. */
export interface ChatHistoryOptions {
  channel: ChatChannel;
  /**
   * Sends one history read. Its identity names the request owner: a new
   * function reads the window again, because the owner it replaces cancels
   * the reads it started. Pass a stable function for one owner.
   */
  request: (action: ChatAction) => Promise<ChatReply>;
  /** Raised by the inbox when the channel may have changed. */
  revision?: number;
  /** The window the owner holds for the channel, or `null` for none. */
  history?: HistoryWindow | null;
  /**
   * The channel's newest position as the inbox last published it, or `null`
   * when nothing states one. A revision moves for reasons other than arriving
   * content — a conservative degraded projection, a read authority change, a
   * preview whose message was rewritten — and a read on another device leaves
   * the position where it was. The tail is gated on this having passed the
   * held window, so those revisions cost no round trip. Any increase
   * publishes a new revision with it, so a gated revision cannot hide one.
   *
   * `null` disables the gate, and is what a caller passes when the projection
   * cannot state the position: a degraded host stops advancing the
   * conversation rows the position is derived from, so gating on them would
   * hold the thread at what it last read. The caller rate-limits the
   * revisions it raises in that state instead.
   */
  position?: string | null;
  /**
   * The read pointer to open the channel at, or `null` to open it at the
   * newest page. Read once, when the thread mounts: the first load is then an
   * `anchor` read that starts {@link ANCHOR_READ_ROWS} rows above the first
   * unread message.
   */
  anchor?: string | null;
  /** Whether the agent accepts `after` reads for the incremental tail. */
  incremental?: boolean;
  /**
   * Accepts a page. A page merged from several replies, when the agent
   * narrowed a range, arrives with the replies' results in `sources`, so the
   * owner can check each of them rather than the merged object it never
   * returned.
   */
  onAccepted?: AcceptHistory;
  /** The channel's history failed verification. */
  onFatal?: (channel: string) => void;
  /** A read is about to start. */
  onLoading?: (load: HistoryLoad) => void;
}

/**
 * Loads verified pages and retains invalidations while a request is running.
 * `onAccepted`, `onFatal` and `onLoading` are read when they are called, so a
 * caller can pass inline functions without reloading the window on every
 * render, and a read in flight reports to the latest render's callbacks.
 */
export function useChatHistory({
  channel,
  request,
  revision = 0,
  history,
  position = null,
  anchor = null,
  incremental: incrementalHistory = false,
  onAccepted,
  onFatal,
  onLoading,
}: ChatHistoryOptions) {
  const accepted = history?.channel === channel.id ? history : null;
  const messages = accepted?.messages ?? EMPTY_MESSAGES;
  const before = accepted?.before ?? null;
  const missing = accepted
    ? [...accepted.verification.values()].some(Boolean)
    : false;
  const [failure, setFailure] = useState<CommandError | null>(null);
  // Errors keep their code beside their message, so the thread can choose
  // how to present a cause rather than treating every failure alike.
  const setError = useCallback(
    (cause: unknown) => setFailure(cause ? normalizeCommandError(cause) : null),
    [],
  );
  const error = failure?.message ?? '';
  const [busy, setBusy] = useState(false);
  const active = useRef(false);
  const reading = useRef(false);
  const again = useRef(false);
  const seenRevision = useRef(revision);
  const generation = useRef(0);
  const heldEnd = useRef(endOf(accepted));
  heldEnd.current = endOf(accepted);
  // The callbacks are read through a ref for the same reason as the position
  // below: in the load's dependencies, an inline callback would reopen the
  // thread on every render. The request is read through it too, so that
  // `read` keeps its identity; a read captures the request when it starts,
  // and the mount effect still depends on `request`, because a new request
  // is a new owner.
  const callbacks = useRef({ request, onAccepted, onFatal, onLoading });
  callbacks.current = { request, onAccepted, onFatal, onLoading };
  // Read through a ref: a position in the load's dependencies would rebuild
  // the callback, and the mount effect depends on it, so every unread count
  // would reopen the thread with a full page.
  // Named for what it is: the newest sequence the inbox published, not the
  // newest one a reply carried.
  const publishedHead = useRef(position);
  publishedHead.current = position;
  // Cleared once the anchored page is accepted, or when Jump to latest
  // replaces it: a later run of the mount effect (a new request owner) then
  // reads the tail, unless this mount detached the window.
  const opening = useRef(anchor);
  // Whether this mount accepted a page. A window detached by an earlier visit
  // is replaced by the tail when the thread opens; one this mount detached is
  // kept when the load is rebuilt.
  const acceptedHere = useRef(false);
  const [anchoring, setAnchoring] = useState(anchor !== null);
  // The read that failed last, for Retry, and the read a rebuilt load
  // cancelled, for the mount effect to repeat.
  const failed = useRef<HistoryRead | null>(null);
  const inFlight = useRef<HistoryRead | null>(null);
  const read = useCallback(
    async (load: HistoryLoad, cursor: string | null = null) => {
      // Opening at the first unread message and jumping to the newest page
      // replace the window, so they supersede a read in flight rather than
      // wait for it.
      if (load === 'anchor' || load === 'latest') {
        generation.current++;
        reading.current = false;
        again.current = false;
      }
      if (load === 'latest') opening.current = null;
      if (reading.current) {
        if (load === 'tail' || load === 'incremental') again.current = true;
        return;
      }
      const run = generation.current;
      const current = () => active.current && generation.current === run;
      // The request names the owner the read started under; the other
      // callbacks are read from the ref where they are called.
      const { request } = callbacks.current;
      const held =
        heldEnd.current?.channel === channel.id ? heldEnd.current : null;
      const tail = held?.tail;
      if (load === 'incremental' && held?.detached) {
        // A detached window ends below the published head, so an `after` read
        // would always report a gap: truncation and rollback both look like
        // that. It pages forward explicitly instead, and reads `after` again
        // only once the inbox publishes a head at or below its tail, which is
        // a rollback or the head it has already reached.
        if (
          publishedHead.current === null ||
          BigInt(publishedHead.current) > BigInt(tail ?? '0')
        )
          return;
      }
      const after =
        load === 'incremental' && incrementalHistory && held ? tail : undefined;
      // Nothing sits beyond the held window: the revision that asked for this
      // read moved for something other than an arrival. A tail would ask the
      // server for rows after its own newest sequence and be told none exist.
      if (
        after !== undefined &&
        !held?.detached &&
        publishedHead.current !== null &&
        BigInt(publishedHead.current) <= BigInt(after)
      )
        return;
      let before: string | null = null;
      if (load === 'older') before = cursor;
      else if (load === 'anchor' && cursor !== null)
        before = anchorBefore(cursor);
      else if (load === 'newer') {
        if (tail === undefined) return;
        before = String(BigInt(tail) + 1n + BigInt(CHAT_PAGE_ROWS));
      }
      reading.current = true;
      inFlight.current = { load, cursor };
      setBusy(true);
      // Read marking waits for the anchored page, as it does on open.
      if (load === 'anchor') setAnchoring(true);
      setError(null);
      callbacks.current.onLoading?.(load);
      // The page load is timed under a hash of the channel, with the row
      // count: a thread open, and the reload each incoming message costs.
      const end = diagnosticLog.span('chat.history', {
        scope: `chan#${hashId(channel.id)}`,
        attrs: {
          older: load === 'older',
          incremental: after !== undefined,
          load,
        },
      });
      try {
        let reply = await request({
          action: 'history',
          channel: channel.id,
          before,
          ...(after === undefined ? {} : { after }),
        });
        let replace = load === 'anchor' || load === 'latest';
        if (!current()) {
          end('cancelled');
          return;
        }
        if (
          after !== undefined &&
          reply.result.kind === 'history' &&
          reply.result.gap !== false
        ) {
          replace = true;
          reply = await request({
            action: 'history',
            channel: channel.id,
            before: null,
          });
        }
        // The agent narrows a range that exceeds its limits from the bottom
        // and keeps the requested end. Older reads below the page fill what
        // that left out, so an anchored window holds the first unread row and
        // a newer page continues the held tail; otherwise the rows between
        // would be skipped, and read once a later row is seen.
        const floor =
          load === 'anchor' && cursor !== null
            ? BigInt(cursor)
            : load === 'newer' && tail !== undefined
              ? BigInt(tail)
              : null;
        let sources: ChatResult[] | undefined;
        if (floor !== null && reply.result.kind === 'history') {
          let page = reply.result;
          let low = lowest(page.messages);
          while (page.before !== null && low !== null && low > floor + 1n) {
            const more = await request({
              action: 'history',
              channel: channel.id,
              before: String(low),
            });
            if (!current()) {
              end('cancelled');
              return;
            }
            const next =
              more.result.kind === 'history'
                ? lowest(more.result.messages)
                : null;
            if (more.result.kind !== 'history' || next === null || next >= low)
              break;
            sources = [...(sources ?? [reply.result]), more.result];
            page = {
              ...page,
              messages: [...more.result.messages, ...page.messages],
              before: more.result.before,
              missing_predecessors: [
                ...more.result.missing_predecessors,
                ...page.missing_predecessors,
              ],
            };
            low = next;
          }
          reply = { ...reply, result: page };
        }
        // An empty range page: the head is below the range, or the agent
        // narrowed it to sequences above the head. The rows after the floor
        // are read directly instead, as an incremental read reads them, and a
        // gap again means the newest page. This also ends forward paging
        // toward a published head the history cannot reach.
        let requested = before;
        if (
          floor !== null &&
          reply.result.kind === 'history' &&
          !reply.result.messages.length
        ) {
          requested = null;
          sources = undefined;
          reply = await request({
            action: 'history',
            channel: channel.id,
            before: null,
            ...(incrementalHistory ? { after: String(floor) } : {}),
          });
          if (!current()) {
            end('cancelled');
            return;
          }
          if (
            !incrementalHistory ||
            (reply.result.kind === 'history' && reply.result.gap !== false)
          ) {
            replace = true;
            if (incrementalHistory)
              reply = await request({
                action: 'history',
                channel: channel.id,
                before: null,
              });
          }
        }
        end('ok', {
          attrs: {
            rows:
              reply.result.kind === 'history'
                ? reply.result.messages.length
                : 0,
          },
        });
        if (!current()) return;
        if (reply.result.kind === 'history') {
          const page = reply.result;
          const kept = heldEnd.current;
          const keptTail = BigInt(kept?.tail ?? '0');
          if (before === null && (after === undefined || replace))
            replace ||= newest(page.messages) < keptTail;
          // A range read decides whether the window reaches the head: the
          // published head says so when there is one. Without it, a page
          // that ends short of the requested end ran out of rows, and one
          // that reaches it may have more beyond.
          let detached: boolean | undefined;
          if (requested !== null && (load === 'anchor' || load === 'newer')) {
            const reached = newest(
              page.messages,
              load === 'newer' ? keptTail : 0n,
            );
            detached =
              publishedHead.current === null
                ? reached >= BigInt(requested) - 1n
                : BigInt(publishedHead.current) > reached;
          }
          const stored = callbacks.current.onAccepted?.(
            page,
            requested,
            {
              replace,
              ...(load === 'newer' ? { newer: true } : {}),
              ...(detached === undefined ? {} : { detached }),
            },
            sources,
          );
          if (load === 'anchor') opening.current = null;
          acceptedHere.current = true;
          // Acceptance publishes synchronously, but React may not render before a
          // retained invalidation starts. Keep the held end current here too:
          // the end of the window the owner accepted, or, when the owner does
          // not return one, the newest of the page and the held tail.
          heldEnd.current = stored
            ? endOf(stored)
            : {
                channel: channel.id,
                tail: tailOf(page.messages, replace ? undefined : kept?.tail),
                detached:
                  detached ??
                  (load === 'older' ? (kept?.detached ?? false) : false),
              };
        }
        failed.current = null;
      } catch (e) {
        const code = normalizeCommandError(e).code;
        end(outcomeForCode(code), { code });
        if (current()) {
          // An incremental read can be skipped by its own gates, so Retry
          // reads the tail.
          failed.current =
            load === 'incremental'
              ? { load: 'tail', cursor: null }
              : { load, cursor };
          if (normalizeCommandError(e).code === 'chat-channel-integrity')
            callbacks.current.onFatal?.(channel.id);
          setError(e);
        }
      } finally {
        if (current()) {
          reading.current = false;
          inFlight.current = null;
          setBusy(false);
          if (load === 'anchor' || load === 'latest') setAnchoring(false);
          if (again.current) {
            again.current = false;
            void read('incremental');
          }
        }
      }
    },
    [channel.id, setError, incrementalHistory],
  );
  const load = useCallback(
    (older: string | null = null, incremental = false) =>
      read(older ? 'older' : incremental ? 'incremental' : 'tail', older),
    [read],
  );
  /** Pages the window forward from its tail; see {@link HistoryLoad}. */
  const loadNewer = useCallback(() => read('newer'), [read]);
  /** Replaces the window with the newest page. */
  const loadLatest = useCallback(() => read('latest'), [read]);
  /** Reopens the window at the first message after `readThrough`. */
  const reopen = useCallback(
    (readThrough: string) => read('anchor', readThrough),
    [read],
  );
  /** Repeats the read that failed last, or reads the tail. */
  const retry = useCallback(() => {
    const last = failed.current;
    return last ? read(last.load, last.cursor) : read('tail');
  }, [read]);
  useEffect(() => {
    generation.current++;
    reading.current = false;
    again.current = false;
    active.current = true;
    const held =
      heldEnd.current?.channel === channel.id ? heldEnd.current : null;
    const cancelled = inFlight.current;
    inFlight.current = null;
    if (channel.readable) {
      if (opening.current !== null) void read('anchor', opening.current);
      // A page read the rebuild cancelled is read again; a tail or
      // incremental read is covered by the tail below.
      else if (
        cancelled &&
        cancelled.load !== 'tail' &&
        cancelled.load !== 'incremental'
      )
        void read(cancelled.load, cancelled.cursor);
      // A window this mount detached stays where the reader left it and
      // pages on demand: the newest page would not continue it, and would
      // replace it. The read the rebuild cancelled no longer clears `busy`.
      else if (held?.detached && acceptedHere.current) setBusy(false);
      else void read('tail');
    }
    return () => {
      active.current = false;
    };
    // `request` is read through a ref, but a new one is a new request owner.
  }, [read, channel.id, channel.readable, request]);
  useEffect(() => {
    if (revision === seenRevision.current) return;
    seenRevision.current = revision;
    if (active.current && channel.readable) void read('incremental');
  }, [channel.readable, read, revision]);
  return {
    messages,
    before,
    missing,
    error,
    failure,
    setError,
    busy,
    loaded: accepted !== null,
    initialLoading: !accepted && !error,
    /** The held window ends below the published head. */
    detached: accepted?.detached ?? false,
    /** The anchored first page has not been accepted or failed yet. */
    anchoring: anchoring && channel.readable,
    load,
    loadNewer,
    loadLatest,
    reopen,
    retry,
  };
}
