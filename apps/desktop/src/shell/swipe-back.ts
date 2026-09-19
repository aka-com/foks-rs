/**
 * Implements two-finger swipe-back navigation from horizontal wheel events.
 *
 * Native webview history gestures are disabled because the application uses
 * `history.replaceState` and maintains its own navigation state. A completed
 * gesture navigates to `parentLocation`, matching the topbar Back action.
 */

import type { Location } from '../location';

/** How long a gap between wheel events ends a gesture, in milliseconds. */
export const GESTURE_GAP_MS = 120;

/** How far the fingers travel, in CSS pixels, before the page changes. */
export const BACK_DISTANCE_PX = 120;

/**
 * Required idle interval before accepting another gesture. This prevents
 * momentum events from triggering a second navigation.
 */
export const QUIET_MS = 300;

export interface SwipeBackOptions {
  /**
   * Where a back swipe goes, read when the gesture completes. `null` makes
   * the gesture inert — a tab's root, chat and first run have no parent.
   */
  target: () => Location | null;
  /**
   * Whether the shell is in a state that accepts the gesture: false while a
   * blocking state or a dialog owns the window.
   */
  enabled: () => boolean;
  navigate: (location: Location) => void;
  /** What the wheel events are read from. Defaults to `window`. */
  root?: EventTarget;
  /** The clock the gesture is timed on. Defaults to `performance.now`. */
  now?: () => number;
}

/**
 * Whether a horizontally scrollable element can consume the event in the
 * gesture direction. When it can, shell navigation does not run.
 */
function absorbsScroll(target: EventTarget | null, deltaX: number): boolean {
  let node = target instanceof Element ? target : null;
  for (; node; node = node.parentElement) {
    if (node.scrollWidth <= node.clientWidth) continue;
    const overflow =
      node.ownerDocument.defaultView?.getComputedStyle(node).overflowX;
    if (overflow !== 'auto' && overflow !== 'scroll') continue;
    const absorbs =
      deltaX < 0
        ? node.scrollLeft > 0
        : node.scrollLeft < node.scrollWidth - node.clientWidth;
    if (absorbs) return true;
  }
  return false;
}

/**
 * Accumulates horizontal movement and suppresses momentum after navigation.
 * An injected clock makes gesture timing deterministic in tests.
 */
export class SwipeBackTracker {
  private readonly options: SwipeBackOptions;
  private readonly now: () => number;
  /** Back-directed distance accumulated in the current gesture, in pixels. */
  private traveled = 0;
  /** When the last wheel event was read, or null before the first one. */
  private lastEventAt: number | null = null;
  /** Set once a gesture is decided: the rest of its events are ignored. */
  private spent = false;

  constructor(options: SwipeBackOptions) {
    this.options = options;
    this.now = options.now ?? (() => performance.now());
  }

  /** Processes one wheel event and navigates when the back-swipe threshold is reached. */
  wheel(event: WheelEvent): void {
    // Pinch-zoom arrives as a ctrl-wheel on macOS, and any other modifier
    // means the reader is driving something that is not navigation.
    if (event.ctrlKey || event.metaKey || event.altKey || event.shiftKey)
      return;
    const at = this.now();
    const gap = this.lastEventAt === null ? Infinity : at - this.lastEventAt;
    this.lastEventAt = at;
    if (this.spent) {
      // Momentum from the decided gesture. Only a quiet stream re-arms, and
      // every event in it postpones that quiet.
      if (gap <= QUIET_MS) return;
      this.spent = false;
      this.traveled = 0;
    } else if (gap > GESTURE_GAP_MS) {
      this.traveled = 0;
    }
    // A vertical scroll that carries a little sideways drift is not a swipe.
    if (Math.abs(event.deltaX) <= 2 * Math.abs(event.deltaY)) {
      this.traveled = 0;
      return;
    }
    if (absorbsScroll(event.target, event.deltaX)) {
      this.traveled = 0;
      return;
    }
    // Two fingers moving right report a negative `deltaX`; that is back.
    // Clamping at zero keeps a forward swipe from counting as back distance
    // when the fingers reverse.
    this.traveled = Math.max(0, this.traveled - event.deltaX);
    if (this.traveled < BACK_DISTANCE_PX) return;
    // Consume the gesture after it reaches the threshold, even when navigation is
    // disabled. Set the latch before calling `navigate` so exceptions cannot allow
    // subsequent momentum events to retry navigation.
    this.spent = true;
    this.traveled = 0;
    const destination = this.options.enabled() ? this.options.target() : null;
    if (destination) this.options.navigate(destination);
  }
}

/**
 * Reads the swipe from the wheel events reaching `root`. The returned function
 * unsubscribes.
 */
export function mountSwipeBack(options: SwipeBackOptions): () => void {
  const root = options.root ?? window;
  const tracker = new SwipeBackTracker(options);
  const onWheel = (event: Event): void => {
    tracker.wheel(event as WheelEvent);
  };
  // The listener never calls `preventDefault`: a swipe that turns out to be a
  // scroll must still scroll.
  root.addEventListener('wheel', onWheel, { passive: true });
  return () => {
    root.removeEventListener('wheel', onWheel);
  };
}
