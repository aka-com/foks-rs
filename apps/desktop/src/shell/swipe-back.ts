/** Bidirectional history gestures with reversible feedback and idle completion.
 * WheelEvent has no portable finger-release or momentum phase, so a quiet
 * stream ends the gesture. No navigation occurs while events keep arriving.
 */
import type { Location } from '../location';

export const GESTURE_GAP_MS = 120;
export const BACK_DISTANCE_PX = 120;
export type HistoryDirection = 'back' | 'forward';
export interface SwipeProgress {
  direction: HistoryDirection;
  progress: number;
}
export interface SwipeBackOptions {
  target: (direction: HistoryDirection) => Location | null;
  enabled: () => boolean;
  navigate: (location: Location, direction: HistoryDirection) => void;
  onProgress?: (progress: SwipeProgress | null) => void;
  root?: EventTarget;
  /** Schedule completion; injectable for deterministic gesture tests. */
  schedule?: (finish: () => void) => () => void;
}

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

export class SwipeBackTracker {
  private direction: HistoryDirection | null = null;
  private destination: Location | null = null;
  private traveled = 0;
  private suppressed = false;
  private cancelTimer?: () => void;

  constructor(private readonly options: SwipeBackOptions) {}

  /** Cancel an in-flight gesture if another input navigates or owns the page. */
  cancel(): void {
    if (this.cancelTimer) this.suppressed = true;
    this.options.onProgress?.(null);
  }

  dispose(): void {
    this.cancelTimer?.();
    this.cancelTimer = undefined;
    this.reset();
  }

  private reset(): void {
    this.direction = null;
    this.destination = null;
    this.traveled = 0;
    this.suppressed = false;
    this.options.onProgress?.(null);
  }

  private finish = (): void => {
    const { direction, destination, traveled, suppressed } = this;
    this.cancelTimer = undefined;
    this.reset();
    if (
      !suppressed &&
      direction &&
      destination &&
      traveled >= BACK_DISTANCE_PX &&
      this.options.enabled() &&
      this.options.target(direction) === destination
    ) {
      this.options.navigate(destination, direction);
    }
  };

  wheel(event: WheelEvent): void {
    this.cancelTimer?.();
    const schedule =
      this.options.schedule ??
      ((finish: () => void) => {
        const timer = setTimeout(finish, GESTURE_GAP_MS);
        return () => clearTimeout(timer);
      });
    this.cancelTimer = schedule(this.finish);
    // A scroll or modified gesture owns its entire stream, even after it
    // reaches an edge. It must not become navigation partway through.
    if (
      event.defaultPrevented ||
      event.ctrlKey ||
      event.metaKey ||
      event.altKey ||
      event.shiftKey ||
      !this.options.enabled() ||
      ((event.deltaX !== 0 || event.deltaY !== 0) &&
        Math.abs(event.deltaX) <= 2 * Math.abs(event.deltaY)) ||
      absorbsScroll(event.target, event.deltaX)
    ) {
      this.cancel();
    }
    if (this.suppressed || (event.deltaX === 0 && event.deltaY === 0)) return;
    if (!this.direction) {
      this.direction = event.deltaX < 0 ? 'back' : 'forward';
      this.destination = this.options.target(this.direction);
      if (!this.destination) {
        this.cancel();
        return;
      }
    }
    // Normalize line/page deltas; trackpads normally report CSS pixels.
    const scale = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? 800 : 1;
    const movement =
      event.deltaX * scale * (this.direction === 'back' ? -1 : 1);
    // Capping progress makes reversing a completed preview cancel promptly,
    // even after a long swipe. The direction stays fixed until idle.
    this.traveled = Math.max(
      0,
      Math.min(BACK_DISTANCE_PX, this.traveled + movement),
    );
    this.options.onProgress?.({
      direction: this.direction,
      progress: this.traveled / BACK_DISTANCE_PX,
    });
  }
}

export function mountSwipeBack(
  options: SwipeBackOptions,
): (() => void) & { cancel: () => void } {
  const root = options.root ?? window;
  const tracker = new SwipeBackTracker(options);
  const onWheel = (event: Event): void => tracker.wheel(event as WheelEvent);
  const onBlur = (): void => tracker.cancel();
  root.addEventListener('wheel', onWheel, { passive: true });
  root.addEventListener('blur', onBlur);
  const stop = () => {
    root.removeEventListener('wheel', onWheel);
    root.removeEventListener('blur', onBlur);
    tracker.dispose();
  };
  stop.cancel = () => tracker.cancel();
  return stop;
}
