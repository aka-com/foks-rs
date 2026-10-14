import { focusedWindow } from './visibility';
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from 'react';
import type { RefObject } from 'react';
import type { ChatMessage } from '../chat-contract';
import { normalizeCommandError } from '../bridge';

/** How long a row has to stay on screen before it counts as read. */
export const SEEN_DWELL_MS = 500;
/** The share of a row, or of the list's height, that has to be on screen. */
const SEEN_SHARE = 0.5;
const THRESHOLDS = [0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1];

const max = (a: string, b: string) => (BigInt(a) >= BigInt(b) ? a : b);

/**
 * Read intent comes from a focused timeline, never from a filtered thread
 * pane. A message counts as read once its row has been at least half on
 * screen (or covered half the list) for {@link SEEN_DWELL_MS} while the
 * window has focus, and the highest such sequence is what `markRead`
 * receives. The server pointer only moves forward, so marking in steps is
 * safe. An engine without `IntersectionObserver` falls back to the newest
 * row while the list is scrolled to the bottom, where that row is on screen.
 *
 * Returns where the new messages begin, until the reader dismisses them,
 * and, when a read mark failed in a way a retry cannot clear, what the
 * server said: the native side stages the read before it asks, and the next
 * synchronization notes that the mark will retry, so a failure a retry can
 * clear, or a conversation that closed meanwhile, is not the thread's to
 * announce.
 */
export function useChatReadIntent(
  channelId: string,
  messages: readonly ChatMessage[],
  atBottom: boolean,
  readThrough: string | null,
  markRead: (channel: string, sequence: string) => Promise<void>,
  enabled: boolean,
  scroller?: RefObject<HTMLElement | null>,
  /**
   * Where the new messages begin, when a staged read has moved it past
   * `readThrough`. Marks are still measured from `readThrough`.
   */
  newAfter: string | null = readThrough,
): {
  newFrom: string | null;
  readError: string;
  /** Marks through `sequence` now, without waiting for it to be seen. */
  markThrough: (sequence: string) => void;
  /** Removes the NEW divider for the rest of this visit. */
  dismissNew: () => void;
} {
  const [newFrom, setNewFrom] = useState<string | null>(null);
  const dismissed = useRef(false);
  const [readFailure, setReadFailure] = useState<{
    channel: string;
    message: string;
  } | null>(null);
  const readMark = readThrough ?? '0';
  const markedThrough = useRef(readMark);
  const markingThrough = useRef('0');
  const seen = useRef('0');
  // Observer and timer callbacks outlive a render; they read these.
  const live = useRef({ channelId, markRead, enabled });
  useLayoutEffect(() => {
    live.current = { channelId, markRead, enabled };
  });
  const disposed = useRef(false);
  useEffect(() => {
    if (newAfter !== null && !dismissed.current)
      setNewFrom((old) => old ?? newAfter);
  }, [newAfter]);
  useEffect(() => {
    if (BigInt(readMark) > BigInt(markedThrough.current))
      markedThrough.current = readMark;
  }, [readMark]);
  useEffect(() => {
    disposed.current = false;
    return () => {
      disposed.current = true;
    };
  }, []);
  const send = useCallback((sequence: string) => {
    if (
      BigInt(sequence) <= BigInt(markedThrough.current) ||
      BigInt(sequence) <= BigInt(markingThrough.current)
    )
      return;
    const { channelId: channel, markRead: mark } = live.current;
    markingThrough.current = sequence;
    void mark(channel, sequence)
      .then(() => {
        if (disposed.current) return;
        markedThrough.current = max(markedThrough.current, sequence);
        setReadFailure(null);
      })
      .catch((cause) => {
        if (disposed.current) return;
        const typed = normalizeCommandError(cause);
        if (typed.code === 'cancelled' || typed.retryable) return;
        setReadFailure({ channel, message: typed.message });
      })
      .finally(() => {
        if (markingThrough.current === sequence) markingThrough.current = '0';
      });
  }, []);
  /**
   * Records `sequence` as seen and sends the mark, when `enabled` is true and
   * the window has focus.
   */
  const saw = useCallback(
    (sequence: string) => {
      if (!live.current.enabled || !focusedWindow()) return;
      seen.current = max(seen.current, sequence);
      send(seen.current);
    },
    [send],
  );
  // The observer outlives renders, so an arrival does not restart a dwell in
  // progress; these reach it from later effects.
  const rearm = useRef<() => void>(() => {});
  const observeRows = useRef<() => void>(() => {});
  useEffect(() => {
    const root = scroller?.current;
    if (!root || typeof IntersectionObserver === 'undefined') return;
    // Rows on screen, with the time each began its dwell, or `null` once it
    // has been read, or while `enabled` is false or the window lacks focus.
    // One timer serves the earliest deadline, so rows that came on screen
    // together are read with one mark.
    const visible = new Map<HTMLElement, number | null>();
    const observed = new Set<HTMLElement>();
    let timer: ReturnType<typeof setTimeout> | undefined;
    const settle = () => {
      clearTimeout(timer);
      timer = undefined;
      const now = performance.now();
      let highest: string | null = null;
      let next = Infinity;
      for (const [row, since] of visible) {
        if (since === null) continue;
        if (now - since < SEEN_DWELL_MS) {
          next = Math.min(next, since + SEEN_DWELL_MS);
          continue;
        }
        visible.set(row, null);
        const sequence = row.dataset.sequence;
        if (sequence)
          highest = highest === null ? sequence : max(highest, sequence);
      }
      if (highest !== null) saw(highest);
      if (next !== Infinity) timer = setTimeout(settle, next - now);
    };
    const dwell = (rows: Iterable<HTMLElement>) => {
      const reading = live.current.enabled && focusedWindow();
      const now = performance.now();
      for (const row of rows) visible.set(row, reading ? now : null);
      settle();
    };
    // A row many times taller than the list crosses no threshold while it
    // scrolls through, so intersecting rows that fell short of the share, or
    // are taller than the list, are measured again on scroll.
    const measured = new Set<HTMLElement>();
    let frame: number | undefined;
    const measure = () => {
      frame = undefined;
      const clip = root.getBoundingClientRect();
      if (clip.height <= 0) return;
      const shown: HTMLElement[] = [];
      for (const row of measured) {
        const rect = row.getBoundingClientRect();
        const overlap =
          Math.min(rect.bottom, clip.bottom) - Math.max(rect.top, clip.top);
        if (overlap < clip.height * SEEN_SHARE) visible.delete(row);
        else if (!visible.has(row)) shown.push(row);
      }
      dwell(shown);
    };
    const onScroll = () => {
      if (frame === undefined && measured.size)
        frame = requestAnimationFrame(measure);
    };
    root.addEventListener('scroll', onScroll, { passive: true });
    const observer = new IntersectionObserver(
      (entries) => {
        const shown: HTMLElement[] = [];
        for (const entry of entries) {
          const row = entry.target as HTMLElement;
          const list = entry.rootBounds?.height ?? Infinity;
          if (!entry.isIntersecting) {
            measured.delete(row);
            visible.delete(row);
            continue;
          }
          const meets =
            entry.intersectionRatio >= SEEN_SHARE ||
            entry.intersectionRect.height >= list * SEEN_SHARE;
          if (!meets || entry.boundingClientRect.height > list)
            measured.add(row);
          else measured.delete(row);
          if (!meets) visible.delete(row);
          else if (!visible.has(row)) shown.push(row);
        }
        dwell(shown);
      },
      { root, threshold: THRESHOLDS },
    );
    observeRows.current = () => {
      const rows = new Set(
        root.querySelectorAll<HTMLElement>('[data-sequence]'),
      );
      for (const row of observed)
        if (!rows.has(row)) {
          observer.unobserve(row);
          observed.delete(row);
          measured.delete(row);
          visible.delete(row);
        }
      for (const row of rows)
        if (!observed.has(row)) {
          observer.observe(row);
          observed.add(row);
        }
    };
    // Regaining focus, or `enabled` turning true, restarts the dwell of every
    // row on screen; losing focus or `enabled` stops it.
    rearm.current = () => dwell([...visible.keys()]);
    observeRows.current();
    return () => {
      observer.disconnect();
      root.removeEventListener('scroll', onScroll);
      if (frame !== undefined) cancelAnimationFrame(frame);
      clearTimeout(timer);
      observeRows.current = () => {};
      rearm.current = () => {};
    };
  }, [saw, scroller]);
  useEffect(() => {
    observeRows.current();
  }, [messages]);
  useEffect(() => {
    rearm.current();
    const onChange = () => rearm.current();
    window.addEventListener('focus', onChange);
    window.addEventListener('blur', onChange);
    document.addEventListener('visibilitychange', onChange);
    return () => {
      window.removeEventListener('focus', onChange);
      window.removeEventListener('blur', onChange);
      document.removeEventListener('visibilitychange', onChange);
    };
  }, [enabled]);
  // Without an observer, only the newest row can be known to be on screen,
  // and only while the list is at the bottom.
  useEffect(() => {
    if (
      (scroller?.current && typeof IntersectionObserver !== 'undefined') ||
      !enabled ||
      !atBottom
    )
      return;
    const latest = messages.at(-1)?.sequence;
    if (!latest) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const schedule = () => {
      clearTimeout(timer);
      if (!focusedWindow()) return;
      timer = setTimeout(() => saw(latest), SEEN_DWELL_MS);
    };
    schedule();
    window.addEventListener('focus', schedule);
    window.addEventListener('blur', schedule);
    document.addEventListener('visibilitychange', schedule);
    return () => {
      clearTimeout(timer);
      window.removeEventListener('focus', schedule);
      window.removeEventListener('blur', schedule);
      document.removeEventListener('visibilitychange', schedule);
    };
  }, [atBottom, enabled, messages, saw, scroller]);
  const dismissNew = useCallback(() => {
    dismissed.current = true;
    setNewFrom(null);
  }, []);
  const markThrough = useCallback(
    (sequence: string) => {
      seen.current = max(seen.current, sequence);
      send(seen.current);
    },
    [send],
  );
  return {
    newFrom,
    readError: readFailure?.channel === channelId ? readFailure.message : '',
    markThrough,
    dismissNew,
  };
}
