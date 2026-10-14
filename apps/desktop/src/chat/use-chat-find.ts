import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from 'react';
import type { RefObject } from 'react';
import { normalizeCommandError } from '../bridge';
import type { CommandError } from '../bridge';
import type { ChatAction, ChatMessage, ChatReply } from '../chat-contract';
import {
  FIND_SCAN_PAGES,
  findRanges,
  foldFindText,
  paintFindHighlights,
  scanPage,
  type FindMatch,
  type FindReference,
} from './find';

/** How long a stepped-to row is marked after it scrolls into view. */
const FLASH_MS = 800;

/**
 * The scan of older pages for the current query. Each run reads up to
 * {@link FIND_SCAN_PAGES} pages from where the last one stopped; a new query
 * starts over.
 */
export interface OlderScan {
  /**
   * `idle` before the first run, `scanning` while one reads, `done` when a
   * run read its pages or reached the start of the channel, `stopped` when
   * the reader cancelled one, and `failed` when a page could not be read.
   */
  state: 'idle' | 'scanning' | 'done' | 'stopped' | 'failed';
  /**
   * The `before` cursor of the next page: `undefined` before the first run,
   * which starts below the held window, and `null` once a page reached the
   * start of the channel.
   */
  cursor: string | null | undefined;
  /** Pages the current or last run read. */
  pages: number;
  /** Rows read across every run for the query. */
  scanned: number;
  /** The insertion time of the oldest row read, if any. */
  reached: string | null;
  /** Matching rows, newest first. */
  hits: FindReference[];
  failure: CommandError | null;
}

const IDLE_SCAN: OlderScan = {
  state: 'idle',
  cursor: undefined,
  pages: 0,
  scanned: 0,
  reached: null,
  hits: [],
  failure: null,
};

/** What {@link useChatFind} reads the thread through. */
export interface ChatFindOptions {
  channel: string;
  /** The message list, whose history rows are matched. */
  scroller: RefObject<HTMLElement | null>;
  /** The held window's rows: matches are measured again when they change. */
  messages: readonly ChatMessage[];
  /** The window's older cursor, where a scan of older pages starts. */
  before: string | null;
  /** Sends a history read for the scan; the conversation's guarded request. */
  request: (action: ChatAction) => Promise<ChatReply>;
  /** Called after find scrolls the list, so the viewport measures again. */
  onScroll?: () => void;
}

/**
 * Find in the open channel: the query, its matches in the rows the list
 * draws, which of them is current, and the scan of older pages.
 *
 * Matches are measured in the list's DOM after each change of query or rows,
 * and again when the rows' text changes, so the count is what the reader
 * sees. The current match is numbered from the newest: Enter steps to an
 * older match and Shift+Enter to a newer one. The current row carries
 * `data-find="current"` and the other matching rows `data-find="hit"`; the
 * matched text itself is drawn with the CSS Custom Highlight API where the
 * webview has it. Find never changes the rows' own DOM.
 */
export function useChatFind({
  channel,
  scroller,
  messages,
  before,
  request,
  onScroll,
}: ChatFindOptions) {
  const [open, setOpen] = useState(false);
  const [query, setQueryText] = useState('');
  // Read by the handlers, which may run before the render that follows them.
  const openRef = useRef(false);
  const queryRef = useRef('');
  const needle = open ? foldFindText(query.trim()) : '';
  const [count, setCount] = useState({ total: 0, current: 0 });
  const [scan, setScanState] = useState<OlderScan>(IDLE_SCAN);
  const scanRef = useRef(scan);
  const setScan = useCallback((next: OlderScan) => {
    scanRef.current = next;
    setScanState(next);
  }, []);
  const scanRun = useRef(0);
  const input = useRef<HTMLInputElement>(null);
  const returnTo = useRef<HTMLElement | null>(null);
  const focusing = useRef<'select' | 'end' | null>(null);
  // The matches as last measured, oldest first, and the current one by its
  // row and its place in the row, which survive a measurement.
  const matches = useRef<FindMatch[]>([]);
  const currentIndex = useRef<number | null>(null);
  const currentKey = useRef<{ row: string; nth: number } | null>(null);
  // A row whose first match becomes current at the next measurement: a found
  // older row, once the page that holds it is drawn.
  const preferred = useRef<string | null>(null);
  const flashed = useRef<{ row: HTMLElement; timer: number } | null>(null);
  const live = useRef({ before, request, onScroll });
  useLayoutEffect(() => {
    live.current = { before, request, onScroll };
  });

  const unflash = useCallback(() => {
    const last = flashed.current;
    if (!last) return;
    clearTimeout(last.timer);
    last.row.removeAttribute('data-find-flash');
    flashed.current = null;
  }, []);
  /**
   * Marks the match at `index` current, with the rows and highlights that go
   * with it; `scroll` centres it in the list and `flash` marks its row.
   */
  const draw = useCallback(
    (index: number | null, scroll: boolean, flash: boolean) => {
      const found = matches.current;
      const root = scroller.current;
      for (const row of root?.querySelectorAll('[data-find]') ?? [])
        row.removeAttribute('data-find');
      const current = index === null ? null : (found[index] ?? null);
      for (const match of found) match.row.setAttribute('data-find', 'hit');
      current?.row.setAttribute('data-find', 'current');
      paintFindHighlights(found, current);
      currentIndex.current = current ? index : null;
      currentKey.current = current
        ? {
            row: current.row.dataset.message ?? '',
            nth: found
              .slice(0, index ?? 0)
              .filter((match) => match.row === current.row).length,
          }
        : null;
      const total = found.length;
      const position = current && index !== null ? total - index : 0;
      setCount((last) =>
        last.total === total && last.current === position
          ? last
          : { total, current: position },
      );
      if (!current || !root) return;
      if (scroll) {
        const box = root.getBoundingClientRect();
        // A webview without Range geometry centres the row instead.
        const target = (
          typeof current.range.getBoundingClientRect === 'function'
            ? current.range
            : current.row
        ).getBoundingClientRect();
        root.scrollTop +=
          target.top - box.top - (box.height - target.height) / 2;
        live.current.onScroll?.();
      }
      if (flash) {
        unflash();
        const row = current.row;
        // Set again after a reflow, so the animation restarts on every step.
        void row.offsetWidth;
        row.setAttribute('data-find-flash', '');
        flashed.current = {
          row,
          timer: window.setTimeout(() => {
            row.removeAttribute('data-find-flash');
            if (flashed.current?.row === row) flashed.current = null;
          }, FLASH_MS),
        };
      }
    },
    [scroller, unflash],
  );
  /**
   * Measures the matches again. The current match stays current while it is
   * still drawn; otherwise the newest becomes current.
   */
  const measure = useCallback(
    (scroll: boolean) => {
      const root = scroller.current;
      const found = root && needle ? findRanges(root, needle) : [];
      matches.current = found;
      let index: number | null = null;
      if (found.length) {
        const prefer = preferred.current;
        if (prefer !== null) {
          const at = found.findIndex((m) => m.row.dataset.message === prefer);
          if (at >= 0) {
            index = at;
            preferred.current = null;
          }
        }
        const key = currentKey.current;
        if (index === null && key) {
          let nth = 0;
          for (let at = 0; at < found.length && index === null; at++) {
            if (found[at].row.dataset.message !== key.row) continue;
            if (nth++ === key.nth) index = at;
          }
        }
        index ??= found.length - 1;
      }
      draw(index, scroll, false);
    },
    [draw, needle, scroller],
  );
  // A new query starts from its newest match and scrolls to it; new rows keep
  // the current match where the reader left it.
  const measuredNeedle = useRef('');
  useLayoutEffect(() => {
    const changed = measuredNeedle.current !== needle;
    measuredNeedle.current = needle;
    if (changed) currentKey.current = null;
    measure(changed);
  }, [measure, messages, needle]);
  // Rows redraw their text without a change of window, such as a mention
  // that resolves once the roster arrives; that is measured once per frame.
  useEffect(() => {
    const root = scroller.current;
    if (!needle || !root || typeof MutationObserver === 'undefined') return;
    let frame: number | null = null;
    const observer = new MutationObserver(() => {
      frame ??= requestAnimationFrame(() => {
        frame = null;
        measure(false);
      });
    });
    observer.observe(root, {
      childList: true,
      subtree: true,
      characterData: true,
    });
    return () => {
      observer.disconnect();
      if (frame !== null) cancelAnimationFrame(frame);
    };
  }, [measure, needle, scroller]);
  // The bar takes focus once it is drawn.
  useLayoutEffect(() => {
    const field = input.current;
    const mode = focusing.current;
    if (!open || !field || !mode) return;
    focusing.current = null;
    field.focus();
    if (mode === 'select') field.select();
    else field.setSelectionRange(field.value.length, field.value.length);
  });
  // Nothing find drew outlives the thread.
  useEffect(
    () => () => {
      scanRun.current++;
      unflash();
      paintFindHighlights([], null);
    },
    [unflash],
  );

  /** Sets the query; a different needle drops the scan of the last one. */
  const setQuery = useCallback(
    (text: string) => {
      if (foldFindText(queryRef.current.trim()) !== foldFindText(text.trim())) {
        scanRun.current++;
        setScan(IDLE_SCAN);
      }
      queryRef.current = text;
      setQueryText(text);
    },
    [setScan],
  );
  /**
   * Opens the bar and focuses its field: with `text`, as the query, the
   * caret after it; without, selecting the query already there.
   */
  const show = useCallback(
    (text?: string) => {
      if (!openRef.current) {
        const active = document.activeElement;
        returnTo.current = active instanceof HTMLElement ? active : null;
      }
      openRef.current = true;
      setOpen(true);
      const changed = text !== undefined && text !== queryRef.current;
      if (text !== undefined) setQuery(text);
      // A field already drawn with its text is focused now; otherwise the
      // layout effect above focuses it once the render that draws it lands.
      const field = input.current;
      if (field && !changed) {
        field.focus();
        if (text === undefined) field.select();
        return;
      }
      focusing.current = text === undefined ? 'select' : 'end';
    },
    [setQuery],
  );
  /**
   * Closes the bar, drops the query and its scan, and returns focus to where
   * it was when the bar opened, or to the list.
   */
  const close = useCallback(() => {
    scanRun.current++;
    openRef.current = false;
    setOpen(false);
    setQuery('');
    setScan(IDLE_SCAN);
    unflash();
    const back = returnTo.current;
    returnTo.current = null;
    if (back?.isConnected && back !== document.body && back !== input.current)
      back.focus({ preventScroll: true });
    else scroller.current?.focus({ preventScroll: true });
  }, [scroller, setQuery, setScan, unflash]);
  /** Steps to the next older or newer match, wrapping at either end. */
  const step = useCallback(
    (direction: 'older' | 'newer') => {
      const total = matches.current.length;
      if (!total) return;
      const at = currentIndex.current ?? total - 1;
      draw(
        direction === 'older' ? (at - 1 + total) % total : (at + 1) % total,
        true,
        true,
      );
    },
    [draw],
  );
  /**
   * Makes the first match in the row of message `id` current: now, when the
   * row is drawn, else once a later measurement draws it.
   */
  const selectRow = useCallback(
    (id: string) => {
      const at = matches.current.findIndex((m) => m.row.dataset.message === id);
      if (at >= 0) draw(at, false, false);
      else preferred.current = id;
    },
    [draw],
  );
  /**
   * Reads up to {@link FIND_SCAN_PAGES} older pages from where the last run
   * stopped, below the held window at first, and keeps a reference to each
   * matching row. The window itself is not changed.
   */
  const searchOlder = useCallback(async () => {
    const text = queryRef.current;
    const folded = foldFindText(text.trim());
    const start = scanRef.current;
    let cursor =
      start.cursor === undefined ? live.current.before : start.cursor;
    if (!folded || cursor === null || start.state === 'scanning') return;
    const run = ++scanRun.current;
    let next: OlderScan = {
      ...start,
      state: 'scanning',
      pages: 0,
      failure: null,
    };
    setScan(next);
    const seen = new Set(start.hits.map((hit) => hit.id));
    while (next.pages < FIND_SCAN_PAGES && cursor !== null) {
      let reply: ChatReply;
      try {
        reply = await live.current.request({
          action: 'history',
          channel,
          before: cursor,
        });
      } catch (cause) {
        if (run === scanRun.current)
          setScan({
            ...next,
            state: 'failed',
            failure: normalizeCommandError(cause),
          });
        return;
      }
      if (run !== scanRun.current) return;
      if (reply.result.kind !== 'history') break;
      const page = reply.result;
      const oldest = page.messages.reduce<ChatMessage | null>(
        (low, m) =>
          low === null || BigInt(m.sequence) < BigInt(low.sequence) ? m : low,
        null,
      );
      // A cursor that does not move below the one read would read the same
      // page again; the scan ends there as at the start of the channel.
      const below: string | null =
        page.before !== null && BigInt(page.before) < BigInt(cursor)
          ? page.before
          : null;
      const found = scanPage(page.messages, text.trim()).filter(
        (hit) => !seen.has(hit.id),
      );
      for (const hit of found) seen.add(hit.id);
      next = {
        ...next,
        cursor: below,
        pages: next.pages + 1,
        scanned: next.scanned + page.messages.length,
        reached: oldest?.insert_time ?? next.reached,
        hits: [...next.hits, ...found],
      };
      cursor = below;
      if (next.pages < FIND_SCAN_PAGES && cursor !== null) setScan(next);
    }
    setScan({ ...next, state: 'done' });
  }, [channel, setScan]);
  /** Stops a scan; the page in flight is discarded and read again next run. */
  const stopOlder = useCallback(() => {
    const current = scanRef.current;
    if (current.state !== 'scanning') return;
    scanRun.current++;
    setScan({
      ...current,
      state:
        current.pages > 0 ? 'stopped' : current.scanned > 0 ? 'done' : 'idle',
    });
  }, [setScan]);
  return {
    open,
    query,
    /** Matches drawn in the list. */
    total: count.total,
    /** The current match, numbered from the newest; 0 when there is none. */
    current: count.current,
    scan,
    input,
    show,
    close,
    setQuery,
    step,
    selectRow,
    searchOlder,
    stopOlder,
  };
}
