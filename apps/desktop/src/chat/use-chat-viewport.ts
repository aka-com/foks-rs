import { useCallback, useLayoutEffect, useRef, useState } from 'react';
import type { ChatMessage } from '../chat-contract';
import type { HistoryLoad } from './use-chat-history';

/** Where a revealed row sits, as a fraction of the list's height from the top. */
const REVEAL_OFFSET = 0.3;

/**
 * DOM measurements are separate from request ownership and accepted history.
 *
 * `anchorTarget` names the row an anchored open scrolls to, the NEW divider
 * or the first unread row; when it returns `null` the list opens at the
 * bottom as a tail load does.
 */
export function useChatViewport(
  messages: readonly ChatMessage[],
  pendingKey = '',
  anchorTarget?: () => HTMLElement | null,
) {
  const [atBottom, setAtBottom] = useState(true);
  const scroller = useRef<HTMLDivElement>(null);
  const followBottom = useRef(true);
  // The rows held when a read that places the list began: an older page, an
  // anchored open, a newer page or Jump to latest. The placement applies to
  // the page that replaces them, not to the next render of any kind, and a
  // tail or incremental read that starts in between leaves it alone.
  const placing = useRef<readonly ChatMessage[] | null>(null);
  const anchoring = useRef(false);
  const placingNewer = useRef(false);
  const held = useRef(messages);
  held.current = messages;
  const target = useRef(anchorTarget);
  target.current = anchorTarget;
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
  /** Scrolls `row` to {@link REVEAL_OFFSET} of the list's height. */
  const reveal = useCallback(
    (row: HTMLElement) => {
      const element = scroller.current;
      if (!element) return;
      element.scrollTop = Math.max(
        0,
        element.scrollTop +
          row.getBoundingClientRect().top -
          element.getBoundingClientRect().top -
          element.clientHeight * REVEAL_OFFSET,
      );
      onScroll();
    },
    [onScroll],
  );
  const capture = useCallback((load: HistoryLoad) => {
    const ordinary = load === 'tail' || load === 'incremental';
    if (ordinary && placing.current !== null) return;
    const element = scroller.current;
    const near =
      !element ||
      element.scrollHeight - element.scrollTop - element.clientHeight < 80;
    placing.current = ordinary ? null : held.current;
    anchoring.current = load === 'anchor';
    placingNewer.current = load === 'newer';
    scroll.current = null;
    // A newer page lands below the rows on screen, so the offset holds; the
    // newest page asked for by Jump to latest is read from the bottom.
    followBottom.current = load === 'latest' || (ordinary && near);
    if (load === 'older') {
      setAtBottom(false);
      if (element)
        scroll.current = {
          height: element.scrollHeight,
          top: element.scrollTop,
        };
    }
  }, []);
  /** Drops a placement whose read failed, so a later page does not apply it. */
  const release = useCallback(() => {
    placing.current = null;
    anchoring.current = false;
    placingNewer.current = false;
    scroll.current = null;
  }, []);
  useLayoutEffect(() => {
    const element = scroller.current;
    if (!element) return;
    if (placing.current === messages) return;
    if (placing.current !== null) {
      const placed = placing.current;
      const newer = placingNewer.current;
      placing.current = null;
      if (scroll.current) {
        element.scrollTop =
          scroll.current.top + element.scrollHeight - scroll.current.height;
        scroll.current = null;
      } else if (anchoring.current) {
        anchoring.current = false;
        const row = target.current?.() ?? null;
        if (row) {
          reveal(row);
          return;
        }
        element.scrollTop = element.scrollHeight;
      } else if (followBottom.current) {
        element.scrollTop = element.scrollHeight;
      } else if (
        newer &&
        placed[0] !== undefined &&
        messages[0]?.sequence !== placed[0].sequence
      ) {
        // The newer page replaced the window rather than extending it, so
        // the old offset points at unrelated rows: start at its top.
        element.scrollTop = 0;
      }
    } else if (followBottom.current) {
      element.scrollTop = element.scrollHeight;
    }
    onScroll();
  }, [messages, pendingKey, onScroll, reveal]);
  return { scroller, atBottom, onScroll, capture, release, reveal };
}
