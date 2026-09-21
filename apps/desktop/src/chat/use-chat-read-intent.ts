import { focusedWindow } from './visibility';
import { useEffect, useRef, useState } from 'react';
import type { ChatMessage } from '../chat-contract';
import { normalizeCommandError } from '../bridge';

/**
 * Read intent comes from a focused timeline, never from a filtered thread
 * pane. Returns where the new messages begin and, when a read mark failed in
 * a way a retry cannot clear, what the server said: the native side stages
 * the read before it asks, and the next synchronization notes that the mark
 * will retry, so a failure a retry can clear, or a conversation that closed
 * meanwhile, is not the thread's to announce.
 */
export function useChatReadIntent(
  channelId: string,
  messages: readonly ChatMessage[],
  atBottom: boolean,
  readThrough: string | null,
  markRead: (channel: string, sequence: string) => Promise<void>,
  enabled: boolean,
): { newFrom: string | null; readError: string } {
  const [newFrom, setNewFrom] = useState<string | null>(null);
  const [readFailure, setReadFailure] = useState<{
    channel: string;
    message: string;
  } | null>(null);
  const readMark = readThrough ?? '0';
  const markedThrough = useRef(readMark);
  const markingThrough = useRef('0');
  useEffect(() => {
    if (readThrough !== null) setNewFrom((old) => old ?? readThrough);
  }, [readThrough]);
  useEffect(() => {
    if (BigInt(readMark) > BigInt(markedThrough.current))
      markedThrough.current = readMark;
  }, [readMark]);
  useEffect(() => {
    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const schedule = () => {
      if (timer) clearTimeout(timer);
      const latest = messages.at(-1)?.sequence;
      if (
        !enabled ||
        !latest ||
        !atBottom ||
        !focusedWindow() ||
        BigInt(latest) <= BigInt(markedThrough.current) ||
        BigInt(latest) <= BigInt(markingThrough.current)
      )
        return;
      timer = setTimeout(() => {
        if (
          !enabled ||
          !atBottom ||
          !focusedWindow() ||
          BigInt(latest) <= BigInt(markedThrough.current) ||
          BigInt(latest) <= BigInt(markingThrough.current)
        )
          return;
        markingThrough.current = latest;
        void markRead(channelId, latest)
          .then(() => {
            if (disposed) return;
            markedThrough.current = latest;
            setReadFailure(null);
          })
          .catch((cause) => {
            if (disposed) return;
            const typed = normalizeCommandError(cause);
            if (typed.code === 'cancelled' || typed.retryable) return;
            setReadFailure({ channel: channelId, message: typed.message });
          })
          .finally(() => {
            if (markingThrough.current === latest) markingThrough.current = '0';
          });
      }, 300);
    };
    schedule();
    window.addEventListener('focus', schedule);
    window.addEventListener('blur', schedule);
    document.addEventListener('visibilitychange', schedule);
    return () => {
      disposed = true;
      if (timer) clearTimeout(timer);
      window.removeEventListener('focus', schedule);
      window.removeEventListener('blur', schedule);
      document.removeEventListener('visibilitychange', schedule);
    };
  }, [atBottom, channelId, markRead, messages, enabled]);
  return {
    newFrom,
    readError: readFailure?.channel === channelId ? readFailure.message : '',
  };
}
