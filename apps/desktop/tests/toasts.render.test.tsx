import assert from 'node:assert/strict';
import test, { type TestContext } from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import { installDom } from './lib/dom-harness';

installDom({ url: 'http://localhost/', timers: true, act: true });
let ui: typeof import('@testing-library/react');
let vite: ViteDevServer;
let ToastController: typeof import('../kit/toasts').ToastController;
let ToastProvider: typeof import('../kit/toasts').ToastProvider;
let toastDurationMs: typeof import('../kit/toasts').toastDurationMs;
let TOAST_HISTORY_LENGTH: number;
let MIN_INFO_DURATION_MS: number;
let MIN_WARNING_DURATION_MS: number;
let MAX_DURATION_MS: number;
test.before(async () => {
  ui = await import('@testing-library/react');
  vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
  ({
    ToastController,
    ToastProvider,
    toastDurationMs,
    TOAST_HISTORY_LENGTH,
    MIN_INFO_DURATION_MS,
    MIN_WARNING_DURATION_MS,
    MAX_DURATION_MS,
  } = await vite.ssrLoadModule('/kit/toasts.tsx'));
});
test.afterEach(() => ui.cleanup());
test.after(async () => vite.close());

const regions = (container: HTMLElement) => {
  const host = container.querySelector('#toasts')!;
  return {
    host,
    polite: host.querySelector<HTMLElement>('[aria-live="polite"]')!,
    assertive: host.querySelector<HTMLElement>('[aria-live="assertive"]')!,
  };
};
const messages = (region: Element) =>
  [...region.querySelectorAll('.toast-message')].map(
    (node) => node.textContent,
  );

test('toasts are appended to live regions that exist before the first toast', () => {
  const controller = new ToastController();
  const rendered = ui.render(
    createElement(ToastProvider, { controller, children: null }),
  );
  const { host, polite, assertive } = regions(rendered.container);
  // Both regions are mounted, and empty, before any message is shown.
  assert.equal(host.getAttribute('aria-live'), null);
  assert.equal(polite.getAttribute('role'), 'status');
  assert.equal(assertive.getAttribute('role'), 'alert');
  for (const region of [polite, assertive]) {
    assert.equal(region.children.length, 0);
    assert.equal(region.getAttribute('aria-atomic'), 'false');
    assert.equal(region.getAttribute('aria-relevant'), 'additions text');
  }
  ui.act(() => {
    controller.show('Saved');
    controller.show('Refresh failed', { tone: 'warning' });
    controller.show('Copied');
  });
  assert.equal(regions(rendered.container).polite, polite);
  assert.deepEqual(messages(polite), ['Saved', 'Copied']);
  assert.deepEqual(messages(assertive), ['Refresh failed']);
  // A toast is not a live region of its own inside the region.
  for (const entry of host.querySelectorAll('.toast')) {
    assert.equal(entry.getAttribute('role'), null);
    assert.equal(entry.getAttribute('aria-live'), null);
    assert.equal(entry.getAttribute('aria-atomic'), 'true');
  }
});

test('a message that opens with ⚠ is information unless its caller says warning', () => {
  const controller = new ToastController();
  const rendered = ui.render(
    createElement(ToastProvider, { controller, children: null }),
  );
  ui.act(() => {
    controller.show('⚠ Restore failed');
  });
  const { polite, assertive } = regions(rendered.container);
  assert.deepEqual(messages(polite), ['⚠ Restore failed']);
  assert.deepEqual(messages(assertive), []);
  assert.equal(controller.getHistory()[0].tone, 'info');
});

test('deduplicated actionable warnings move to the assertive region and keep usable controls', () => {
  const controller = new ToastController();
  controller.show('Pending refresh', { dedupeKey: 'refresh' });
  const rendered = ui.render(
    createElement(ToastProvider, { controller, children: null }),
  );
  let actions = 0;
  ui.act(() => {
    controller.show('Refresh failed', {
      dedupeKey: 'refresh',
      tone: 'warning',
      action: {
        label: 'Retry',
        onAction: () => {
          actions++;
        },
      },
    });
  });
  const { polite, assertive } = regions(rendered.container);
  assert.equal(rendered.container.querySelectorAll('.toast').length, 1);
  assert.deepEqual(messages(polite), []);
  assert.deepEqual(messages(assertive), ['Refresh failed']);
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Retry' }));
  assert.equal(actions, 1);
  assert.equal(rendered.container.querySelectorAll('.toast').length, 0);
  ui.act(() => {
    controller.show('Another warning', { tone: 'warning' });
  });
  ui.fireEvent.click(
    rendered.getByRole('button', { name: 'Dismiss notification' }),
  );
  assert.equal(rendered.container.querySelectorAll('.toast').length, 0);
});

test('a toast stays for its reading time, at least 4 s and 8 s for a warning', () => {
  assert.equal(toastDurationMs('Saved', 'info'), MIN_INFO_DURATION_MS);
  assert.equal(toastDurationMs('Saved', 'warning'), MIN_WARNING_DURATION_MS);
  assert.equal(toastDurationMs('x'.repeat(200), 'info'), 13_000);
  assert.equal(toastDurationMs('x'.repeat(200), 'warning'), 13_000);
  assert.equal(toastDurationMs('x'.repeat(2_000), 'info'), MAX_DURATION_MS);
});

/**
 * Replaces the window's timers and `Date.now` with a clock that moves only
 * when `advance` is called, running each timer that falls due in order.
 */
function fakeClock(t: TestContext): { advance: (ms: number) => void } {
  let now = 0;
  let nextId = 1;
  const timers = new Map<number, { at: number; run: () => void }>();
  t.mock.method(Date, 'now', () => now);
  t.mock.method(window, 'setTimeout', ((run: () => void, ms = 0) => {
    const id = nextId++;
    timers.set(id, { at: now + ms, run });
    return id;
  }) as typeof window.setTimeout);
  t.mock.method(window, 'clearTimeout', ((id: number) => {
    timers.delete(id);
  }) as typeof window.clearTimeout);
  return {
    advance(ms) {
      const end = now + ms;
      for (;;) {
        const due = [...timers]
          .filter(([, timer]) => timer.at <= end)
          .sort(([, a], [, b]) => a.at - b.at)[0];
        if (!due) break;
        timers.delete(due[0]);
        now = due[1].at;
        ui.act(() => due[1].run());
      }
      now = end;
    },
  };
}

const EXIT_MS = 300;

test('the countdown holds while a toast is hovered, holds focus or the window is hidden', (t) => {
  const clock = fakeClock(t);
  // jsdom reports a hidden document unless it pretends to be visual.
  let hidden = false;
  Object.defineProperty(document, 'hidden', {
    configurable: true,
    get: () => hidden,
  });
  t.after(() => {
    delete (document as { hidden?: boolean }).hidden;
  });
  const controller = new ToastController();
  const rendered = ui.render(
    createElement(ToastProvider, { controller, children: null }),
  );
  const shown = () =>
    messages(rendered.container.querySelector('#toasts')!).join();
  const toast = () => rendered.container.querySelector<HTMLElement>('.toast')!;

  ui.act(() => {
    controller.show('Saved');
  });
  clock.advance(MIN_INFO_DURATION_MS - 1);
  assert.equal(shown(), 'Saved');
  clock.advance(1 + EXIT_MS);
  assert.equal(shown(), '');

  ui.act(() => {
    controller.show('Restore failed', { tone: 'warning' });
  });
  clock.advance(MIN_WARNING_DURATION_MS - 1);
  assert.equal(shown(), 'Restore failed');
  clock.advance(1 + EXIT_MS);
  assert.equal(shown(), '');

  // Hover: the time left when the pointer arrives is kept.
  ui.act(() => {
    controller.show('Copied');
  });
  clock.advance(1_000);
  ui.fireEvent.mouseEnter(toast());
  clock.advance(60_000);
  assert.equal(shown(), 'Copied');
  ui.fireEvent.mouseLeave(toast());
  clock.advance(MIN_INFO_DURATION_MS - 1_000 - 1);
  assert.equal(shown(), 'Copied');
  clock.advance(1 + EXIT_MS);
  assert.equal(shown(), '');

  // Focus within the toast, as when a keyboard user reaches its button.
  ui.act(() => {
    controller.show('Moved');
  });
  ui.act(() => {
    toast().querySelector<HTMLElement>('[data-toast-dismiss]')!.focus();
  });
  clock.advance(60_000);
  assert.equal(shown(), 'Moved');
  ui.act(() => {
    (document.activeElement as HTMLElement).blur();
  });
  clock.advance(MIN_INFO_DURATION_MS - 1);
  assert.equal(shown(), 'Moved');
  clock.advance(1 + EXIT_MS);
  assert.equal(shown(), '');

  // A hidden window: nothing counts down until it is shown again.
  hidden = true;
  ui.act(() => {
    document.dispatchEvent(new Event('visibilitychange'));
  });
  ui.act(() => {
    controller.show('Synced');
  });
  clock.advance(60_000);
  assert.equal(shown(), 'Synced');
  hidden = false;
  ui.act(() => {
    document.dispatchEvent(new Event('visibilitychange'));
  });
  clock.advance(MIN_INFO_DURATION_MS - 1);
  assert.equal(shown(), 'Synced');
  clock.advance(1 + EXIT_MS);
  assert.equal(shown(), '');
});

test('a repeated message restarts the full duration and is announced again', (t) => {
  const clock = fakeClock(t);
  Object.defineProperty(document, 'hidden', {
    configurable: true,
    get: () => false,
  });
  t.after(() => {
    delete (document as { hidden?: boolean }).hidden;
  });
  const controller = new ToastController();
  const rendered = ui.render(
    createElement(ToastProvider, { controller, children: null }),
  );
  const { polite } = regions(rendered.container);
  ui.act(() => {
    controller.show('Copied');
  });
  const first = polite.querySelector('.toast-message');
  clock.advance(3_000);
  ui.act(() => {
    controller.show('Copied');
  });
  assert.deepEqual(messages(polite), ['Copied']);
  // The text is added to the region again rather than left unchanged.
  assert.notEqual(polite.querySelector('.toast-message'), first);
  clock.advance(MIN_INFO_DURATION_MS - 1);
  assert.deepEqual(messages(polite), ['Copied']);
  clock.advance(1 + EXIT_MS);
  assert.deepEqual(messages(polite), []);
});

test('a hold does not keep a toast past the deadline its caller gives', (t) => {
  const clock = fakeClock(t);
  let hidden = false;
  Object.defineProperty(document, 'hidden', {
    configurable: true,
    get: () => hidden,
  });
  t.after(() => {
    delete (document as { hidden?: boolean }).hidden;
  });
  const controller = new ToastController();
  const rendered = ui.render(
    createElement(ToastProvider, { controller, children: null }),
  );
  const shown = () =>
    messages(rendered.container.querySelector('#toasts')!).join();
  const toast = () => rendered.container.querySelector<HTMLElement>('.toast')!;

  // Hovered: the toast closes at its deadline, after its reading time.
  ui.act(() => {
    controller.show('Password copied · clears in 15 s', {
      expiresAt: Date.now() + 15_000,
    });
  });
  ui.fireEvent.mouseEnter(toast());
  clock.advance(15_000 - 1);
  assert.equal(shown(), 'Password copied · clears in 15 s');
  clock.advance(1 + EXIT_MS);
  assert.equal(shown(), '');

  // Not held: the shorter reading time still applies.
  ui.act(() => {
    controller.show('Password copied · clears in 15 s', {
      expiresAt: Date.now() + 15_000,
    });
  });
  clock.advance(MIN_INFO_DURATION_MS + EXIT_MS);
  assert.equal(shown(), '');

  // A hidden window: an Undo toast still closes when its Undo expires.
  hidden = true;
  ui.act(() => {
    document.dispatchEvent(new Event('visibilitychange'));
  });
  ui.act(() => {
    controller.show('Moved notes to Trash', {
      action: { label: 'Undo', onAction: () => undefined },
      durationMs: 8_000,
      expiresAt: Date.now() + 8_000,
    });
  });
  clock.advance(8_000 + EXIT_MS);
  assert.equal(shown(), '');
});

test('the controller keeps the last messages it showed, after their toasts close, until cleared', () => {
  let now = 1_000;
  const controller = new ToastController(() => now);
  let published = 0;
  const unsubscribe = controller.subscribeHistory(() => {
    published++;
  });
  // Shown before a provider connects, and still recorded.
  controller.show('Saved');
  const rendered = ui.render(
    createElement(ToastProvider, { controller, children: null }),
  );
  ui.act(() => {
    now = 2_000;
    controller.show('Restoring notes…', { dedupeKey: 'restore' });
    now = 3_000;
    controller.show('Restore failed', {
      dedupeKey: 'restore',
      tone: 'warning',
    });
    controller.show('Copied', {
      action: { label: 'Undo', onAction: () => undefined },
    });
  });
  // The outcome replaces its progress message, as the toast itself does.
  assert.deepEqual(
    controller
      .getHistory()
      .map((record) => [record.message, record.tone, record.shownAt]),
    [
      ['Saved', 'info', 1_000],
      ['Restore failed', 'warning', 3_000],
      ['Copied', 'info', 3_000],
    ],
  );
  ui.act(() => {
    for (const node of rendered.container.querySelectorAll<HTMLElement>(
      '[data-toast-dismiss]',
    ))
      node.click();
  });
  assert.equal(rendered.container.querySelectorAll('.toast').length, 0);
  assert.equal(controller.getHistory().length, 3);

  ui.act(() => {
    for (let index = 0; index < TOAST_HISTORY_LENGTH + 5; index++)
      controller.show(`Message ${index}`);
  });
  const history = controller.getHistory();
  assert.equal(history.length, TOAST_HISTORY_LENGTH);
  assert.equal(history[0].message, 'Message 5');
  assert.equal(
    history[TOAST_HISTORY_LENGTH - 1].message,
    `Message ${TOAST_HISTORY_LENGTH + 4}`,
  );

  const before = published;
  controller.clearHistory();
  assert.deepEqual(controller.getHistory(), []);
  assert.equal(published, before + 1);
  unsubscribe();
});
