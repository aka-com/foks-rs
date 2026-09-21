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
import { diagnosticLog, hashId } from '../diagnostics/log';

const EMPTY_MESSAGES: ChatMessage[] = [];

/** Loads verified pages and retains invalidations while a request is running. */
export function useChatHistory(
  channel: ChatChannel,
  request: (action: ChatAction) => Promise<ChatReply>,
  revision = 0,
  onAccepted?: (
    page: Extract<ChatResult, { kind: 'history' }>,
    before: string | null,
  ) => void,
  history?: HistoryWindow | null,
  onFatal?: (channel: string) => void,
  onLoading?: (before: string | null) => void,
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
  const load = useCallback(
    async (older: string | null = null) => {
      if (reading.current) {
        if (!older) again.current = true;
        return;
      }
      reading.current = true;
      setBusy(true);
      setError(null);
      onLoading?.(older);
      // The page load is timed under a hash of the channel, with the row
      // count: a thread open, and the reload each incoming message costs.
      const end = diagnosticLog.span('chat.history', {
        scope: `chan#${hashId(channel.id)}`,
        attrs: { older: older !== null },
      });
      try {
        const reply = await request({
          action: 'history',
          channel: channel.id,
          before: older,
        });
        end('ok', {
          attrs: {
            rows:
              reply.result.kind === 'history'
                ? reply.result.messages.length
                : 0,
          },
        });
        if (!active.current) return;
        if (reply.result.kind === 'history') {
          const page = reply.result;
          onAccepted?.(page, older);
        }
      } catch (e) {
        const code = normalizeCommandError(e).code;
        end(code === 'cancelled' ? 'cancelled' : 'error', { code });
        if (active.current) {
          if (normalizeCommandError(e).code === 'chat-channel-integrity')
            onFatal?.(channel.id);
          setError(e);
        }
      } finally {
        reading.current = false;
        if (active.current) {
          setBusy(false);
          if (again.current) {
            again.current = false;
            void load();
          }
        }
      }
    },
    [request, channel.id, onAccepted, onFatal, onLoading, setError],
  );
  useEffect(() => {
    active.current = true;
    if (channel.readable) void load();
    return () => {
      active.current = false;
    };
  }, [load, channel.readable]);
  useEffect(() => {
    if (revision === seenRevision.current) return;
    seenRevision.current = revision;
    if (active.current && channel.readable) void load();
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
