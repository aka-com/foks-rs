/** Tests swipe recognition, navigation guards, scroll handling, and momentum suppression. */

import assert from 'node:assert/strict';
import test from 'node:test';

import { installDom } from './lib/dom-harness';
import {
  BACK_DISTANCE_PX,
  SwipeBackTracker,
  mountSwipeBack,
} from '../src/shell/swipe-back';
import type { SwipeBackOptions } from '../src/shell/swipe-back';
import type { Location } from '../src/location';

installDom({ url: 'http://localhost/', body: '<div id="root"></div>' });

/** The destination the fixtures navigate to. */
const PARENT: Location = { kind: 'files' };

/** A clock the test moves by hand. */
function clock(): { now: () => number; advance: (ms: number) => void } {
  let value = 0;
  return {
    now: () => value,
    advance: (ms) => {
      value += ms;
    },
  };
}

interface WheelFields {
  deltaX?: number;
  deltaY?: number;
  ctrlKey?: boolean;
  metaKey?: boolean;
  altKey?: boolean;
  shiftKey?: boolean;
  target?: EventTarget | null;
}

/** A wheel event with only the fields the tracker reads. */
function wheel(fields: WheelFields): WheelEvent {
  return {
    deltaX: 0,
    deltaY: 0,
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    shiftKey: false,
    target: null,
    ...fields,
  } as unknown as WheelEvent;
}

interface Rig {
  /** Reads one wheel event, `gap` milliseconds after the last one. */
  swipe: (fields: WheelFields, gap?: number) => void;
  /** Reads enough back deltas to complete a gesture, in 40px steps. */
  fullSwipe: (fields?: WheelFields) => void;
  advance: (ms: number) => void;
  navigations: Location[];
}

function rig(options: Partial<SwipeBackOptions> = {}): Rig {
  const time = clock();
  const navigations: Location[] = [];
  const tracker = new SwipeBackTracker({
    target: () => PARENT,
    enabled: () => true,
    navigate: (location) => navigations.push(location),
    ...options,
    now: time.now,
  });
  const swipe = (fields: WheelFields, gap = 20): void => {
    time.advance(gap);
    tracker.wheel(wheel(fields));
  };
  return {
    swipe,
    fullSwipe: (fields = {}) => {
      for (let sent = 0; sent < BACK_DISTANCE_PX; sent += 40)
        swipe({ deltaX: -40, ...fields });
    },
    advance: time.advance,
    navigations,
  };
}

/* ------------------------------------------------------- the back swipe -- */

test('a back swipe past the distance opens the parent once', () => {
  const { swipe, navigations } = rig();
  swipe({ deltaX: -50 });
  swipe({ deltaX: -50 });
  assert.deepEqual(navigations, []);
  swipe({ deltaX: -50 });
  assert.deepEqual(navigations, [PARENT]);
});

test('the distance is reached exactly, not overshot', () => {
  const { swipe, navigations } = rig();
  swipe({ deltaX: -60 });
  swipe({ deltaX: -59 });
  assert.equal(navigations.length, 0);
  swipe({ deltaX: -1 });
  assert.equal(navigations.length, 1);
});

test('a forward swipe does not navigate', () => {
  const { swipe, navigations } = rig();
  for (let sent = 0; sent < 400; sent += 40) swipe({ deltaX: 40 });
  assert.deepEqual(navigations, []);
});

test('reversing into a back swipe spends the forward distance first', () => {
  const { swipe, navigations } = rig();
  swipe({ deltaX: 100 });
  swipe({ deltaX: -100 });
  assert.deepEqual(navigations, []);
  swipe({ deltaX: -20 });
  assert.equal(navigations.length, 1);
});

/* ----------------------------------------------------------- what it is -- */

test('a mostly vertical scroll is not a swipe', () => {
  const { swipe, navigations } = rig();
  for (let sent = 0; sent < 400; sent += 40) swipe({ deltaX: -40, deltaY: 20 });
  assert.deepEqual(navigations, []);
});

test('a sideways drift under a vertical scroll is not a swipe', () => {
  const { swipe, navigations } = rig();
  for (let sent = 0; sent < 400; sent += 40) swipe({ deltaX: -40, deltaY: 60 });
  assert.deepEqual(navigations, []);
});

test('a dominant sideways delta is a swipe', () => {
  const { swipe, navigations } = rig();
  for (let sent = 0; sent < BACK_DISTANCE_PX; sent += 40)
    swipe({ deltaX: -40, deltaY: 19 });
  assert.equal(navigations.length, 1);
});

test('a pinch-zoom is not a swipe', () => {
  const { fullSwipe, navigations } = rig();
  fullSwipe({ ctrlKey: true });
  assert.deepEqual(navigations, []);
});

test('a modified wheel is not a swipe', () => {
  for (const modifier of ['metaKey', 'altKey', 'shiftKey'] as const) {
    const { fullSwipe, navigations } = rig();
    fullSwipe({ [modifier]: true });
    assert.deepEqual(navigations, [], modifier);
  }
});

/* ------------------------------------------------------------- the gaps -- */

test('a gap longer than the gesture window restarts the distance', () => {
  const { swipe, navigations } = rig();
  swipe({ deltaX: -60 });
  swipe({ deltaX: -60 }, 121);
  assert.deepEqual(navigations, []);
  swipe({ deltaX: -60 }, 120);
  assert.equal(navigations.length, 1);
});

test('momentum after a swipe does not open the grandparent', () => {
  const { fullSwipe, swipe, navigations } = rig();
  fullSwipe();
  assert.equal(navigations.length, 1);
  for (let sent = 0; sent < 800; sent += 40) swipe({ deltaX: -40 }, 50);
  assert.equal(navigations.length, 1);
});

test('pausing scroll events resets the swipe gesture', () => {
  const { fullSwipe, advance, navigations } = rig();
  fullSwipe();
  advance(301);
  fullSwipe();
  assert.equal(navigations.length, 2);
});

/* ---------------------------------------------------------- the guards -- */

test('a swipe does not navigate when the current location has no parent', () => {
  const { fullSwipe, navigations } = rig({ target: () => null });
  fullSwipe();
  assert.deepEqual(navigations, []);
});

test('a disabled gesture does not navigate or reactivate before the idle interval', () => {
  let enabled = false;
  const { fullSwipe, swipe, advance, navigations } = rig({
    enabled: () => enabled,
  });
  fullSwipe();
  assert.deepEqual(navigations, []);
  // The declined gesture is spent: re-enabling mid-swipe does not revive it.
  enabled = true;
  for (let sent = 0; sent < 400; sent += 40) swipe({ deltaX: -40 });
  assert.deepEqual(navigations, []);
  advance(301);
  fullSwipe();
  assert.equal(navigations.length, 1);
});

test('a navigate that throws does not leave the gesture armed', () => {
  const time = clock();
  let calls = 0;
  const tracker = new SwipeBackTracker({
    target: () => PARENT,
    enabled: () => true,
    navigate: () => {
      calls += 1;
      throw new Error('navigation failed');
    },
    now: time.now,
  });
  const push = (): void => {
    time.advance(20);
    tracker.wheel(wheel({ deltaX: -BACK_DISTANCE_PX }));
  };
  assert.throws(push, /navigation failed/);
  push();
  push();
  assert.equal(calls, 1);
});

/* ------------------------------------------------- the scrollable panes -- */

/** A pane wide enough to scroll sideways, holding one child. */
function scroller(scrollLeft: number): HTMLElement {
  const pane = document.createElement('div');
  pane.style.overflowX = 'auto';
  Object.defineProperties(pane, {
    scrollWidth: { configurable: true, value: 800 },
    clientWidth: { configurable: true, value: 400 },
    scrollLeft: { configurable: true, value: scrollLeft },
  });
  const inner = document.createElement('div');
  pane.append(inner);
  document.body.append(pane);
  return inner;
}

test('a pane scrolled away from its left edge keeps the swipe', () => {
  const { fullSwipe, navigations } = rig();
  fullSwipe({ target: scroller(300) });
  assert.deepEqual(navigations, []);
});

test('a pane at its left edge passes the swipe up to the shell', () => {
  const { fullSwipe, navigations } = rig();
  fullSwipe({ target: scroller(0) });
  assert.equal(navigations.length, 1);
});

test('a pane that cannot scroll sideways passes the swipe up', () => {
  const pane = document.createElement('div');
  pane.style.overflowX = 'hidden';
  Object.defineProperties(pane, {
    scrollWidth: { configurable: true, value: 800 },
    clientWidth: { configurable: true, value: 400 },
    scrollLeft: { configurable: true, value: 300 },
  });
  const inner = document.createElement('div');
  pane.append(inner);
  document.body.append(pane);
  const { fullSwipe, navigations } = rig();
  fullSwipe({ target: inner });
  assert.equal(navigations.length, 1);
});

/* ------------------------------------------------------------ the mount -- */

test('the mounted listener reads the events and stops on unsubscribe', () => {
  const time = clock();
  const navigations: Location[] = [];
  const root = document.createElement('div');
  const inner = document.createElement('div');
  root.append(inner);
  document.body.append(root);
  const stop = mountSwipeBack({
    target: () => PARENT,
    enabled: () => true,
    navigate: (location) => navigations.push(location),
    root,
    now: time.now,
  });
  const send = (): void => {
    time.advance(20);
    inner.dispatchEvent(
      new window.WheelEvent('wheel', {
        deltaX: -BACK_DISTANCE_PX,
        bubbles: true,
      }),
    );
  };
  send();
  assert.deepEqual(navigations, [PARENT]);
  time.advance(301);
  stop();
  send();
  assert.equal(navigations.length, 1);
});
