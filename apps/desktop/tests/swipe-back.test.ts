/** Behavior tests for bidirectional history gestures. */

import assert from 'node:assert/strict';
import test from 'node:test';

import { installDom } from './lib/dom-harness';
import { readSource } from './lib/source';
import {
  BACK_DISTANCE_PX,
  SwipeBackTracker,
  mountSwipeBack,
} from '../src/shell/swipe-back';
import type {
  HistoryDirection,
  SwipeBackOptions,
} from '../src/shell/swipe-back';
import type { Location } from '../src/location';

installDom({ url: 'http://localhost/', body: '<div id="root"></div>' });

const DESTINATIONS: Readonly<Record<HistoryDirection, Location>> = {
  back: { kind: 'files' },
  forward: { kind: 'chat' },
};

interface WheelFields {
  defaultPrevented?: boolean;
  deltaX?: number;
  deltaY?: number;
  deltaMode?: number;
  ctrlKey?: boolean;
  metaKey?: boolean;
  altKey?: boolean;
  shiftKey?: boolean;
  target?: EventTarget | null;
}

function wheel(fields: WheelFields): WheelEvent {
  return {
    deltaMode: 0,
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

function scheduler(): {
  schedule: NonNullable<SwipeBackOptions['schedule']>;
  finish: () => void;
} {
  let pending: (() => void) | undefined;
  return {
    schedule: (finish) => {
      pending = finish;
      return () => {
        if (pending === finish) pending = undefined;
      };
    },
    finish: () => {
      const callback = pending;
      pending = undefined;
      callback?.();
    },
  };
}

function rig(options: Partial<SwipeBackOptions> = {}) {
  const timer = scheduler();
  const navigations: Array<{
    location: Location;
    direction: HistoryDirection;
  }> = [];
  const tracker = new SwipeBackTracker({
    target: (direction) => DESTINATIONS[direction],
    enabled: () => true,
    navigate: (location, direction) =>
      navigations.push({ location, direction }),
    schedule: timer.schedule,
    ...options,
  });
  return {
    navigations,
    dispose: () => tracker.dispose(),
    finish: timer.finish,
    swipe: (fields: WheelFields) => tracker.wheel(wheel(fields)),
    complete: (direction: HistoryDirection, fields: WheelFields = {}) => {
      tracker.wheel(
        wheel({
          deltaX: direction === 'back' ? -BACK_DISTANCE_PX : BACK_DISTANCE_PX,
          ...fields,
        }),
      );
      timer.finish();
    },
    cancel: () => tracker.cancel(),
  };
}

test('completed back and forward gestures navigate after the gesture ends', () => {
  for (const direction of ['back', 'forward'] as const) {
    const subject = rig();
    subject.swipe({
      deltaX: direction === 'back' ? -BACK_DISTANCE_PX : BACK_DISTANCE_PX,
    });
    assert.deepEqual(subject.navigations, []);
    subject.finish();
    assert.deepEqual(subject.navigations, [
      { location: DESTINATIONS[direction], direction },
    ]);
  }
});

test('short, reversed, cancelled, and unavailable gestures do not navigate', () => {
  const short = rig();
  short.swipe({ deltaX: -BACK_DISTANCE_PX / 2 });
  short.finish();
  assert.deepEqual(short.navigations, []);

  const reversed = rig();
  reversed.swipe({ deltaX: -BACK_DISTANCE_PX });
  reversed.swipe({ deltaX: BACK_DISTANCE_PX });
  reversed.finish();
  assert.deepEqual(reversed.navigations, []);

  const cancelled = rig();
  cancelled.swipe({ deltaX: -BACK_DISTANCE_PX });
  cancelled.cancel();
  cancelled.finish();
  assert.deepEqual(cancelled.navigations, []);

  const unavailable = rig({ target: () => null });
  unavailable.complete('back');
  assert.deepEqual(unavailable.navigations, []);
});

test('vertical, modified, consumed, and disabled gestures remain inert', () => {
  const cases: Array<{
    fields: WheelFields;
    options?: Partial<SwipeBackOptions>;
  }> = [
    { fields: { deltaX: -BACK_DISTANCE_PX, deltaY: BACK_DISTANCE_PX } },
    { fields: { deltaX: -BACK_DISTANCE_PX, ctrlKey: true } },
    { fields: { deltaX: -BACK_DISTANCE_PX, metaKey: true } },
    { fields: { deltaX: -BACK_DISTANCE_PX, altKey: true } },
    { fields: { deltaX: -BACK_DISTANCE_PX, shiftKey: true } },
    { fields: { deltaX: -BACK_DISTANCE_PX, defaultPrevented: true } },
    {
      fields: { deltaX: -BACK_DISTANCE_PX },
      options: { enabled: () => false },
    },
  ];
  for (const { fields, options } of cases) {
    const subject = rig(options);
    subject.swipe(fields);
    subject.finish();
    assert.deepEqual(subject.navigations, []);
  }
});

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

test('horizontal scrolling takes precedence until the pane reaches its edge', () => {
  const scrolling = rig();
  scrolling.complete('back', { target: scroller(300) });
  assert.deepEqual(scrolling.navigations, []);

  const atEdge = rig();
  atEdge.complete('back', { target: scroller(0) });
  assert.deepEqual(atEdge.navigations, [
    { location: DESTINATIONS.back, direction: 'back' },
  ]);
});

test('the mounted listener stops receiving gestures after cleanup', () => {
  const timer = scheduler();
  const navigations: Location[] = [];
  const root = document.createElement('div');
  const inner = document.createElement('div');
  root.append(inner);
  document.body.append(root);
  const stop = mountSwipeBack({
    target: (direction) => DESTINATIONS[direction],
    enabled: () => true,
    navigate: (location) => navigations.push(location),
    root,
    schedule: timer.schedule,
  });
  const send = (): void => {
    inner.dispatchEvent(
      new window.WheelEvent('wheel', {
        deltaX: -BACK_DISTANCE_PX,
        bubbles: true,
      }),
    );
  };

  send();
  timer.finish();
  assert.deepEqual(navigations, [DESTINATIONS.back]);
  stop();
  send();
  timer.finish();
  assert.deepEqual(navigations, [DESTINATIONS.back]);
});

test('zero-distance wheel events at release do not cancel a completed preview', () => {
  const r = rig();
  r.swipe({ deltaX: -120 });
  r.swipe({ deltaX: 0 });
  r.finish();
  assert.deepEqual(r.navigations, [
    { location: DESTINATIONS.back, direction: 'back' },
  ]);
});

test('losing window focus cancels a pending navigation', () => {
  const root = document.createElement('div');
  let finish: () => void = () => {};
  let navigations = 0;
  const stop = mountSwipeBack({
    root,
    target: () => DESTINATIONS.back,
    enabled: () => true,
    navigate: () => {
      navigations++;
    },
    schedule: (callback) => {
      finish = callback;
      return () => {};
    },
  });
  root.dispatchEvent(new window.WheelEvent('wheel', { deltaX: -120 }));
  root.dispatchEvent(new window.Event('blur'));
  finish();
  assert.equal(navigations, 0);
  stop();
});

test('momentum keeps one reversible preview and the next gesture needs no extra cooldown', () => {
  const progress: unknown[] = [];
  const r = rig({ onProgress: (value) => progress.push(value) });
  r.swipe({ deltaX: -60 });
  assert.deepEqual(progress.at(-1), { direction: 'back', progress: 0.5 });
  for (let i = 0; i < 40; i++) r.swipe({ deltaX: -40 });
  assert.deepEqual(r.navigations, []);
  assert.deepEqual(progress.at(-1), { direction: 'back', progress: 1 });
  r.finish();
  assert.equal(r.navigations.length, 1);
  assert.equal(progress.at(-1), null);
  r.complete('forward');
  assert.equal(r.navigations.length, 2);
});

test('scroll and disabled streams never turn into navigation midway', () => {
  let enabled = false;
  const r = rig({ enabled: () => enabled });
  r.swipe({ deltaX: -120 });
  enabled = true;
  r.swipe({ deltaX: -120 });
  r.finish();
  assert.deepEqual(r.navigations, []);
  for (const sign of [-1, 1]) {
    r.swipe({ deltaX: sign * 120, target: scroller(200) });
    r.swipe({ deltaX: sign * 120, target: scroller(sign < 0 ? 0 : 400) });
    r.finish();
    assert.deepEqual(r.navigations, []);
  }
  r.swipe({ deltaX: -120 });
  enabled = false;
  r.finish();
  assert.deepEqual(r.navigations, []);
});

test('destination changes and disposal cancel delayed work; idle cancellation does not swallow input', () => {
  let destination = DESTINATIONS.back;
  const r = rig({ target: () => destination });
  r.swipe({ deltaX: -120 });
  destination = DESTINATIONS.forward;
  r.finish();
  r.swipe({ deltaX: -120 });
  r.dispose();
  r.finish();
  assert.deepEqual(r.navigations, []);
  r.cancel();
  r.complete('back');
  assert.deepEqual(r.navigations, [
    { location: destination, direction: 'back' },
  ]);
});

test('short gestures remain separate and line deltas are normalized', () => {
  const r = rig();
  r.swipe({ deltaX: -60 });
  r.finish();
  r.swipe({ deltaX: -60 });
  r.finish();
  assert.deepEqual(r.navigations, []);
  r.swipe({ deltaX: -8, deltaMode: 1 });
  r.finish();
  assert.equal(r.navigations.length, 1);
});

test('a throwing navigation leaves the next gesture usable', () => {
  let calls = 0;
  const r = rig({
    navigate: () => {
      calls++;
      throw new Error('failed');
    },
  });
  assert.throws(() => r.complete('back'), /failed/);
  assert.throws(() => r.complete('back'), /failed/);
  assert.equal(calls, 2);
});

test('the viewport and nested scroll containers suppress elastic overscroll without disabling scrolling', async () => {
  const css = await readSource(
    '../src/styles/native-window.css',
    import.meta.url,
  );
  const rule = /(?:^|\n)\*\s*\{([^}]*)\}/.exec(css);
  assert.ok(rule);
  assert.match(rule[1], /overscroll-behavior: none;/);
  assert.doesNotMatch(rule[1], /overflow:\s*hidden|touch-action:\s*none/);
  const style = document.createElement('style');
  style.textContent = rule[0];
  const nested = document.createElement('div');
  nested.className = 'sb';
  nested.style.overflow = 'auto';
  const input = document.createElement('textarea');
  nested.append(input);
  document.head.append(style);
  document.getElementById('root')!.append(nested);
  try {
    for (const element of [
      document.documentElement,
      document.body,
      nested,
      input,
    ]) {
      assert.equal(
        window
          .getComputedStyle(element)
          .getPropertyValue('overscroll-behavior'),
        'none',
      );
    }
    assert.equal(window.getComputedStyle(nested).overflow, 'auto');
  } finally {
    nested.remove();
    style.remove();
  }
});
