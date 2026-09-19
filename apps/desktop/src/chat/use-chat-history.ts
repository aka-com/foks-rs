import { normalizeCommandError } from '../bridge';
import { useCallback, useEffect, useRef, useState } from 'react';
import type {
  ChatAction,
  ChatChannel,
  ChatMessage,
  ChatReply,
  ChatResult,
} from '../chat-contract';
import { failure } from './actions';
import type { HistoryWindow } from './conversation-model';

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
  const [error, setError] = useState('');
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
      setError('');
      onLoading?.(older);
      try {
        const reply = await request({
          action: 'history',
          channel: channel.id,
          before: older,
        });
        if (!active.current) return;
        if (reply.result.kind === 'history') {
          const page = reply.result;
          onAccepted?.(page, older);
        }
      } catch (e) {
        if (active.current) {
          if (normalizeCommandError(e).code === 'chat-channel-integrity')
            onFatal?.(channel.id);
          setError(failure(e));
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
    [request, channel.id, onAccepted, onFatal, onLoading],
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
    setError,
    busy,
    loaded: accepted !== null,
    initialLoading: !accepted && !error,
    load,
  };
}
