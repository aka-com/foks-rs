import { useCallback, useLayoutEffect, useRef, useState } from 'react';
import type { ChatMessage } from '../chat-contract';

/** DOM measurements are separate from request ownership and accepted history. */
export function useChatViewport(
  messages: readonly ChatMessage[],
  pendingKey = '',
) {
  const [atBottom, setAtBottom] = useState(true);
  const scroller = useRef<HTMLDivElement>(null);
  const followBottom = useRef(true);
  const scroll = useRef<{ height: number; top: number } | null>(null);
  const onScroll = useCallback(() => {
    const element = scroller.current;
    if (element) {
      const bottom =
        element.scrollHeight - element.scrollTop - element.clientHeight < 80;
      followBottom.current = bottom;
      setAtBottom(bottom);
    }
  }, []);
  const capture = useCallback((older: string | null) => {
    const element = scroller.current;
    followBottom.current =
      !older &&
      (!element ||
        element.scrollHeight - element.scrollTop - element.clientHeight < 80);
    if (older) setAtBottom(false);
    if (older && element)
      scroll.current = { height: element.scrollHeight, top: element.scrollTop };
  }, []);
  useLayoutEffect(() => {
    if (scroll.current && scroller.current) {
      scroller.current.scrollTop =
        scroll.current.top +
        scroller.current.scrollHeight -
        scroll.current.height;
      scroll.current = null;
    } else if (followBottom.current && scroller.current) {
      scroller.current.scrollTop = scroller.current.scrollHeight;
    }
    onScroll();
  }, [messages, pendingKey, onScroll]);
  return { scroller, atBottom, onScroll, capture };
}
