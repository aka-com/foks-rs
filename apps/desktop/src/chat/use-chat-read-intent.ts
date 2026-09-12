import { focusedWindow } from './visibility';
import { useEffect, useRef, useState } from 'react';
import type { ChatMessage } from '../chat-contract';
import { failure } from './actions';

/** Read intent comes from a focused timeline, never from a filtered thread pane. */
export function useChatReadIntent(
  channelId: string,
  messages: readonly ChatMessage[],
  atBottom: boolean,
  readThrough: string | null,
  markRead: (channel: string, sequence: string) => Promise<void>,
  setError: (message: string) => void,
  enabled: boolean,
) {
  const [newFrom, setNewFrom] = useState<string | null>(null);
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
            if (!disposed) markedThrough.current = latest;
          })
          .catch((cause) => {
            if (!disposed) setError(failure(cause));
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
  }, [atBottom, channelId, markRead, messages, setError, enabled]);
  return newFrom;
}
