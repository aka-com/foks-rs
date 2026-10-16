/**
 * The screens the shell loads on demand: the loading pane while a screen's
 * module loads, one load however often the screen renders or is preloaded, a
 * synchronous render once loaded, a failed load reaching the screen's error
 * boundary, and the preload that a mounted screen router schedules.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { Component, createElement, Suspense } from 'react';
import type { ComponentType, ReactNode } from 'react';
import { installDom } from './lib/dom-harness';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div>',
  timers: true,
  act: true,
});

let ui: typeof import('@testing-library/react');
let deferred: typeof import('../src/app/deferred-screens');
test.before(async () => {
  ui = await import('@testing-library/react');
  deferred = await import('../src/app/deferred-screens');
});
test.afterEach(() => ui.cleanup());

/** A module load the test settles, counting how often it starts. */
function controlledLoad<P extends object>() {
  let resolve!: (screen: ComponentType<P>) => void;
  let reject!: (error: unknown) => void;
  const loaded = new Promise<ComponentType<P>>((settle, fail) => {
    resolve = settle;
    reject = fail;
  });
  const control = {
    starts: 0,
    load: () => {
      control.starts++;
      return loaded;
    },
    resolve: (screen: ComponentType<P>) => resolve(screen),
    reject: (error: unknown) => reject(error),
  };
  return control;
}

function Greeting({ name }: { name: string }): ReactNode {
  return <p>Hello {name}</p>;
}

/**
 * Renders inside an awaited `act`, so a suspension's retry is flushed by the
 * `act` that settles its load.
 */
async function renderAwaited(element: ReactNode) {
  let rendered: ReturnType<typeof ui.render> | undefined;
  await ui.act(async () => {
    rendered = ui.render(element);
  });
  assert.ok(rendered);
  return rendered;
}

function inSuspense(children: ReactNode): ReactNode {
  return <Suspense fallback={<deferred.ScreenLoading />}>{children}</Suspense>;
}

test('a deferred screen shows the loading pane until its module loads, then the screen', async () => {
  const load = controlledLoad<{ name: string }>();
  const { Component: Screen } = deferred.deferredScreen('Greeting', load.load);
  const rendered = await renderAwaited(inSuspense(<Screen name="Ada" />));
  assert.ok(rendered.getByRole('status', { name: 'Loading' }));
  assert.equal(rendered.queryByText('Hello Ada'), null);
  await ui.act(async () => load.resolve(Greeting));
  assert.ok(rendered.getByText('Hello Ada'));
  assert.equal(rendered.queryByRole('status'), null);
  assert.equal(load.starts, 1);
});

test('a loaded screen renders without suspending, and preloads share one load', async () => {
  const load = controlledLoad<{ name: string }>();
  const screen = deferred.deferredScreen('Greeting', load.load);
  const first = screen.preload();
  const second = screen.preload();
  load.resolve(Greeting);
  await Promise.all([first, second]);
  assert.equal(load.starts, 1);
  // Rendered without a Suspense boundary: a screen that suspended here would
  // make React throw instead of drawing it.
  const rendered = ui.render(<screen.Component name="Grace" />);
  assert.ok(rendered.getByText('Hello Grace'));
  rendered.unmount();
  const again = ui.render(inSuspense(<screen.Component name="Linus" />));
  assert.ok(again.getByText('Hello Linus'));
  assert.equal(load.starts, 1);
});

class Boundary extends Component<
  { children: ReactNode },
  { error: string | null }
> {
  override state = { error: null as string | null };
  static getDerivedStateFromError(error: unknown) {
    return { error: String(error) };
  }
  override render(): ReactNode {
    return this.state.error ? (
      <p role="alert">{this.state.error}</p>
    ) : (
      this.props.children
    );
  }
}

test('a failed module load reaches the error boundary rather than retrying', async () => {
  const load = controlledLoad<{ name: string }>();
  const screen = deferred.deferredScreen('Greeting', load.load);
  const Screen = screen.Component;
  // React reports a caught render error on the console; this test expects one.
  const consoleError = console.error;
  console.error = () => undefined;
  try {
    const rendered = await renderAwaited(
      createElement(Boundary, null, inSuspense(<Screen name="Ada" />)),
    );
    await ui.act(async () => load.reject(new Error('chunk unavailable')));
    assert.match(
      rendered.getByRole('alert').textContent ?? '',
      /chunk unavailable/,
    );
    assert.equal(load.starts, 1);
  } finally {
    console.error = consoleError;
  }
});

/**
 * Replaces `name` on the window for the length of `body`, restoring the
 * original property, or its absence, afterwards.
 */
async function withWindowProperty(
  name: string,
  value: unknown,
  body: () => Promise<void>,
): Promise<void> {
  const original = Object.getOwnPropertyDescriptor(window, name);
  Object.defineProperty(window, name, { configurable: true, value });
  try {
    await body();
  } finally {
    if (original) Object.defineProperty(window, name, original);
    else Reflect.deleteProperty(window, name);
  }
}

/** A timer or idle-callback scheduler that records its calls and runs none. */
function recordingScheduler() {
  const scheduled: { callback: () => void; option: unknown }[] = [];
  const cancelled: number[] = [];
  return {
    scheduled,
    cancelled,
    schedule: (callback: () => void, option: unknown) =>
      scheduled.push({ callback, option }),
    cancel: (handle: number) => cancelled.push(handle),
  };
}

type Scheduler = ReturnType<typeof recordingScheduler>;

/** Runs `body` in a webview without `requestIdleCallback`, as WebKitGTK is. */
function withTimers(timers: Scheduler, body: () => Promise<void>) {
  return withWindowProperty('requestIdleCallback', undefined, () =>
    withWindowProperty('setTimeout', timers.schedule, () =>
      withWindowProperty('clearTimeout', timers.cancel, body),
    ),
  );
}

/** Runs `body` in a webview with `requestIdleCallback`. */
function withIdleCallbacks(idle: Scheduler, body: () => Promise<void>) {
  return withWindowProperty('requestIdleCallback', idle.schedule, () =>
    withWindowProperty('cancelIdleCallback', idle.cancel, body),
  );
}

function Preloading({ preload }: { preload: () => Promise<unknown> }) {
  deferred.useIdlePreload(preload);
  return null;
}

test('without requestIdleCallback the preload starts after the fixed delay, once', async () => {
  const timers = recordingScheduler();
  let starts = 0;
  const preload = () => {
    starts++;
    return Promise.resolve();
  };
  await withTimers(timers, async () => {
    const rendered = ui.render(<Preloading preload={preload} />);
    assert.equal(starts, 0, 'the preload waits for the delay');
    assert.equal(timers.scheduled.length, 1);
    assert.equal(timers.scheduled[0].option, deferred.PRELOAD_DELAY_MS);
    timers.scheduled[0].callback();
    assert.equal(starts, 1);
    rendered.rerender(<Preloading preload={preload} />);
    assert.equal(timers.scheduled.length, 1, 'a rerender schedules nothing');
    rendered.unmount();
  });
});

test('with requestIdleCallback the preload runs in an idle period bounded by the delay', async () => {
  const idle = recordingScheduler();
  let starts = 0;
  const preload = () => {
    starts++;
    return Promise.resolve();
  };
  await withIdleCallbacks(idle, async () => {
    const rendered = ui.render(<Preloading preload={preload} />);
    assert.equal(starts, 0);
    assert.deepEqual(
      idle.scheduled.map((call) => call.option),
      [{ timeout: deferred.PRELOAD_DELAY_MS }],
    );
    idle.scheduled[0].callback();
    assert.equal(starts, 1);
    rendered.unmount();
  });
});

test('unmounting before the delay cancels the preload', async () => {
  const timers = recordingScheduler();
  const idle = recordingScheduler();
  const preload = () => Promise.reject(new Error('not expected'));
  await withTimers(timers, async () => {
    ui.render(<Preloading preload={preload} />).unmount();
    assert.deepEqual(timers.cancelled, [1]);
  });
  await withIdleCallbacks(idle, async () => {
    ui.render(<Preloading preload={preload} />).unmount();
    assert.deepEqual(idle.cancelled, [1]);
  });
});

test('a failed preload is swallowed; the render that needs the screen reports it', async () => {
  const timers = recordingScheduler();
  let rejected = false;
  const preload = () =>
    Promise.reject(new Error('chunk unavailable')).finally(() => {
      rejected = true;
    });
  await withTimers(timers, async () => {
    const rendered = ui.render(<Preloading preload={preload} />);
    timers.scheduled[0].callback();
    // An unhandled rejection would fail the test run.
    await new Promise((resolve) => setImmediate(resolve));
    assert.ok(rejected);
    rendered.unmount();
  });
});
