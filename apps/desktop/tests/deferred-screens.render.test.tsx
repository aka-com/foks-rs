/**
 * The screens the shell loads on demand: the loading pane while a screen's
 * module loads, one load however often the screen renders or is preloaded, a
 * synchronous render once loaded, and a failed load reaching the screen's
 * error boundary.
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
