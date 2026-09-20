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
import type { HistoryWindow } from './conversation-model';
import { diagnosticLog, hashId, outcomeForCode } from '../diagnostics/log';

const EMPTY_MESSAGES: ChatMessage[] = [];

/** Loads verified pages and retains invalidations while a request is running. */
export function useChatHistory(
  channel: ChatChannel,
  request: (action: ChatAction) => Promise<ChatReply>,
  revision = 0,
  onAccepted?: (
    page: Extract<ChatResult, { kind: 'history' }>,
    before: string | null,
    replace?: boolean,
  ) => void,
  history?: HistoryWindow | null,
  onFatal?: (channel: string) => void,
  onLoading?: (before: string | null) => void,
  incrementalHistory = false,
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
  position: string | null = null,
) {
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
  const latest = useRef(accepted);
  latest.current = accepted;
  // Read through a ref: a position in the load's dependencies would rebuild
  // the callback, and the mount effect depends on it, so every unread count
  // would reopen the thread with a full page.
  // Named apart from the page-local `head` computed below, which is the
  // newest sequence a reply carried rather than the one the inbox published.
  const publishedHead = useRef(position);
  publishedHead.current = position;
  const load = useCallback(
    async (older: string | null = null, incremental = false) => {
      if (reading.current) {
        if (!older) again.current = true;
        return;
      }
      const run = generation.current;
      const current = () => active.current && generation.current === run;
      const after =
        incremental &&
        incrementalHistory &&
        !older &&
        latest.current?.channel === channel.id
          ? latest.current.messages.at(-1)?.sequence
          : undefined;
      // Nothing sits beyond the held window: the revision that asked for this
      // read moved for something other than an arrival. A tail would ask the
      // server for rows after its own newest sequence and be told none exist.
      if (
        after !== undefined &&
        publishedHead.current !== null &&
        BigInt(publishedHead.current) <= BigInt(after)
      )
        return;
      reading.current = true;
      setBusy(true);
      setError(null);
      onLoading?.(older);
      // The page load is timed under a hash of the channel, with the row
      // count: a thread open, and the reload each incoming message costs.
      const end = diagnosticLog.span('chat.history', {
        scope: `chan#${hashId(channel.id)}`,
        attrs: { older: older !== null, incremental: after !== undefined },
      });
      try {
        let reply = await request({
          action: 'history',
          channel: channel.id,
          before: older,
          ...(after === undefined ? {} : { after }),
        });
        let replace = false;
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
          if (!older && (after === undefined || replace)) {
            const head = page.messages.reduce(
              (max, m) => (BigInt(m.sequence) > max ? BigInt(m.sequence) : max),
              0n,
            );
            replace ||=
              head < BigInt(latest.current?.messages.at(-1)?.sequence ?? '0');
          }
          onAccepted?.(page, older, replace);
          // Acceptance publishes synchronously, but React may not render before a
          // retained invalidation starts. Keep its next cursor current here too.
          const held = replace ? [] : (latest.current?.messages ?? []);
          const rows = new Map(
            [...held, ...page.messages].map((m) => [m.sequence, m]),
          );
          latest.current = {
            channel: channel.id,
            messages: [...rows.values()].sort((a, b) =>
              BigInt(a.sequence) < BigInt(b.sequence) ? -1 : 1,
            ),
            before: page.before,
            verification: new Map(),
          };
        }
      } catch (e) {
        const code = normalizeCommandError(e).code;
        end(outcomeForCode(code), { code });
        if (current()) {
          if (normalizeCommandError(e).code === 'chat-channel-integrity')
            onFatal?.(channel.id);
          setError(e);
        }
      } finally {
        if (current()) {
          reading.current = false;
          setBusy(false);
          if (again.current) {
            again.current = false;
            void load(null, true);
          }
        }
      }
    },
    [
      request,
      channel.id,
      onAccepted,
      onFatal,
      onLoading,
      setError,
      incrementalHistory,
    ],
  );
  useEffect(() => {
    generation.current++;
    reading.current = false;
    again.current = false;
    active.current = true;
    if (channel.readable) void load();
    return () => {
      active.current = false;
    };
  }, [load, channel.readable]);
  useEffect(() => {
    if (revision === seenRevision.current) return;
    seenRevision.current = revision;
    if (active.current && channel.readable) void load(null, true);
  }, [channel.readable, load, revision]);
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
    load,
  };
}
