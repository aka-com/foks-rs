/**
 * Windowing math for a list that scrolls inside an ancestor scroller.
 *
 * Kept free of DOM and React so the arithmetic — which row is the first
 * visible one, how tall the spacers standing in for the unmounted rows must
 * be — is testable on its own. Heights are supplied fully resolved: the caller
 * measures what it has mounted and fills the rest with a content-shaped guess,
 * so a fresh list windows correctly before anything has been measured.
 */

export interface VirtualListInput {
  /** Every row's height in list order, measured or estimated. */
  heights: readonly number[];
  /** The list's top edge in the scroller's content coordinates. */
  listTop: number;
  /** The scroller's current offset. */
  scrollTop: number;
  /** The scroller's visible height. */
  viewport: number;
  /** Rows kept mounted beyond each edge of the visible range. */
  overscan: number;
}

export interface VirtualListWindow {
  /** First mounted row. */
  start: number;
  /** One past the last mounted row. */
  end: number;
  /** Spacer height standing in for the rows before `start`. */
  padTop: number;
  /** Spacer height standing in for the rows from `end` on. */
  padBottom: number;
}

const EMPTY: VirtualListWindow = { start: 0, end: 0, padTop: 0, padBottom: 0 };

/** A height we can lay out with: negative, NaN and Infinity all mean "unknown". */
function usableHeight(height: number | undefined): number {
  return typeof height === 'number' && Number.isFinite(height) && height > 0
    ? height
    : 0;
}

/**
 * The slice of rows to mount, and the spacer heights that keep the scrollbar
 * describing the whole list.
 *
 * Guarantees `padTop + sum(heights[start..end)) + padBottom` equals the full
 * list height, so windowing never changes how far the list scrolls.
 */
export function virtualListWindow(input: VirtualListInput): VirtualListWindow {
  const count = input.heights.length;
  if (count === 0) return EMPTY;

  const offsets = new Array<number>(count);
  let total = 0;
  for (let i = 0; i < count; i += 1) {
    offsets[i] = total;
    total += usableHeight(input.heights[i]);
  }

  const viewport = Math.max(0, input.viewport);
  // Clamping to the list's total height prevents an empty flash when a filter
  // drastically reduces the list size. The scroller will initially report the
  // previous, larger offset, which would render an empty window if left unclamped
  // until the browser naturally adjusts the scroll position.
  const top = Math.min(
    Math.max(0, input.scrollTop - input.listTop),
    Math.max(0, total - viewport),
  );
  const bottom = top + viewport;

  // Determines the first row that isn't fully scrolled out of view at the top,
  // and the last row that begins before the bottom of the viewport. This inclusive
  // range ensures that a zero-height viewport still mounts at least one row.
  let first = 0;
  while (
    first < count - 1 &&
    offsets[first] + usableHeight(input.heights[first]) <= top
  ) {
    first += 1;
  }
  let last = first;
  while (last < count - 1 && offsets[last + 1] < bottom) last += 1;

  const overscan = Math.max(0, Math.floor(input.overscan));
  const start = Math.max(0, first - overscan);
  const end = Math.min(count, last + 1 + overscan);
  return {
    start,
    end,
    padTop: offsets[start],
    padBottom: end < count ? total - offsets[end] : 0,
  };
}
