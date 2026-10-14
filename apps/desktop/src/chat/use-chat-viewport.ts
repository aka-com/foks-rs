import { useCallback, useLayoutEffect, useRef, useState } from 'react';
import type { ChatMessage } from '../chat-contract';
import type { ListPosition } from './history-cache';
import type { HistoryLoad } from './use-chat-history';

/** Where a revealed row sits, as a fraction of the list's height from the top. */
const REVEAL_OFFSET = 0.3;

/**
 * DOM measurements are separate from request ownership and accepted history.
 *
 * `anchorTarget` names the row an anchored open scrolls to, the NEW divider
 * or the first unread row; when it returns `null` the list opens at the
 * bottom as a tail load does. `resume`, read once when the list mounts, is
 * where the reader left the held rows; the list opens there instead of at
 * the bottom.
 *
 * While the reader is away from the bottom, the list keeps the first row on
 * screen where it was when rows above it change: an older page lands above
 * it, or the window drops rows from its oldest end.
 */
export function useChatViewport(
  messages: readonly ChatMessage[],
  pendingKey = '',
  anchorTarget?: () => HTMLElement | null,
  resume: ListPosition | null = null,
) {
  const [atBottom, setAtBottom] = useState(resume === null);
  const scroller = useRef<HTMLDivElement>(null);
  const followBottom = useRef(resume === null);
  const resuming = useRef(resume);
  // The first row on screen and its offset, measured after every placement
  // and scroll. `null` when no row is laid out.
  const measured = useRef<ListPosition | null>(null);
  // The rows the placement effect last ran for.
  const rendered = useRef(messages);
  // The rows held when a read that places the list began: an older page, an
  // anchored open, a newer page or Jump to latest. The placement applies to
  // the page that replaces them, not to the next render of any kind, and a
  // tail or incremental read that starts in between leaves it alone.
  const placing = useRef<readonly ChatMessage[] | null>(null);
  const anchoring = useRef(false);
  const placingNewer = useRef(false);
  const held = useRef(messages);
  const target = useRef(anchorTarget);
  // Declared before the placement effect below, which reads `target` in the
  // same commit.
  useLayoutEffect(() => {
    held.current = messages;
    target.current = anchorTarget;
  });
  const scroll = useRef<{ height: number; top: number } | null>(null);
  const onScroll = useCallback(() => {
    const element = scroller.current;
    if (element) {
      const bottom =
        element.scrollHeight - element.scrollTop - element.clientHeight < 80;
      followBottom.current = bottom;
      measured.current = firstOnScreen(element);
      setAtBottom(bottom);
    }
  }, []);
  /**
   * Scrolls so that the row `position` names sits at its offset again.
   * Returns false when that row is not in the list.
   */
  const keep = useCallback((position: ListPosition | null): boolean => {
    const element = scroller.current;
    const row = element && position && messageRow(element, position.message);
    if (!element || !position || !row) return false;
    element.scrollTop +=
      row.getBoundingClientRect().top -
      element.getBoundingClientRect().top -
      position.offset;
    return true;
  }, []);
  /**
   * Where the reader is leaving the list: the first row on screen, or `null`
   * at the bottom, where the list opens anyway.
   */
  const position = useCallback(
    (): ListPosition | null => (followBottom.current ? null : measured.current),
    [],
  );
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
    const previous = rendered.current;
    rendered.current = messages;
    const resumed = resuming.current;
    resuming.current = null;
    if (placing.current !== null) {
      const placed = placing.current;
      const newer = placingNewer.current;
      placing.current = null;
      if (scroll.current) {
        // The older page lands above the first row on screen, and the trim
        // may drop rows below it, so the row holds its place. Without a
        // laid-out row the added height is taken from the top instead.
        if (!keep(measured.current))
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
      } else if (newer && placed.length > 0) {
        // The newer page replaced the window rather than extending it, so
        // the old offset points at unrelated rows: start at its top. One that
        // extended it may have dropped rows from the oldest end.
        const tail = placed.at(-1)?.id;
        if (!messages.some((m) => m.id === tail)) element.scrollTop = 0;
        else keep(measured.current);
      }
    } else if (resumed !== null && keep(resumed)) {
      // Opened where the reader left the held rows.
    } else if (followBottom.current || resumed !== null) {
      followBottom.current = true;
      element.scrollTop = element.scrollHeight;
    } else if (previous[0]?.id !== messages[0]?.id) {
      // Rows left or joined the oldest end above the reader.
      keep(measured.current);
    }
    onScroll();
  }, [messages, pendingKey, onScroll, reveal, keep]);
  return {
    scroller,
    atBottom,
    onScroll,
    capture,
    release,
    reveal,
    position,
  };
}

/** The message row for `id`, compared by attribute, not by selector. */
function messageRow(root: HTMLElement, id: string): HTMLElement | null {
  for (const row of root.querySelectorAll<HTMLElement>('[data-message]'))
    if (row.dataset.message === id) return row;
  return null;
}

/**
 * The first message row whose bottom is below the top edge of `root`, and
 * the distance from that edge to the row's top. Rows are in document order,
 * so the search halves the list. `null` when no row is laid out.
 */
function firstOnScreen(root: HTMLElement): ListPosition | null {
  const rows = root.querySelectorAll<HTMLElement>('[data-message]');
  const top = root.getBoundingClientRect().top;
  let low = 0;
  let high = rows.length;
  while (low < high) {
    const middle = (low + high) >> 1;
    if (rows[middle].getBoundingClientRect().bottom > top) high = middle;
    else low = middle + 1;
  }
  const row = rows[low];
  const rect = row?.getBoundingClientRect();
  if (!row?.dataset.message || !rect || rect.height <= 0) return null;
  return { message: row.dataset.message, offset: rect.top - top };
}
