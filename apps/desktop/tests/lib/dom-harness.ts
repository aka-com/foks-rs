/**
 * Configures the JSDOM test environment for component render tests.
 *
 * Initializes global browser DOM APIs and mock timers prior to module evaluation.
 * Production builds and tests run without global Tauri IPC objects.
 */

import { JSDOM } from 'jsdom';
import test from 'node:test';

export const nativeSetInterval = globalThis.setInterval;
export const nativeSetTimeout = globalThis.setTimeout;

/**
 * How long, from the start of a test, the `timers: true` keep-alive holds the
 * event loop open. A test still waiting on unreferenced timers or an unsettled
 * promise after this is cancelled by node, together with the rest of its file.
 * The slowest render test takes about 40 seconds on a 4-core machine.
 */
const KEEP_ALIVE_MS = 120_000;

/**
 * Resolves on the next timer tick, once the promises already queued have
 * settled. With `timers: true` the global timers are unreferenced, and the
 * harness's keep-alive timer holds the event loop open for at most
 * `KEEP_ALIVE_MS` per test. This timer is referenced, so the wait does not
 * depend on the keep-alive.
 */
export function settle(): Promise<void> {
  return new Promise((resolve) => {
    nativeSetTimeout(resolve, 0);
  });
}

class TestResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}

export interface InstallDomOptions {
  /** Initial document URL evaluated during module initialization. */
  url?: string;
  /** Inner HTML of `<body>` — the mount points the app boots into. */
  body?: string;
  /**
   * Unref `setInterval`/`setTimeout` so a stray timer cannot hang the run. A
   * referenced keep-alive timer then holds the event loop open for at most
   * `KEEP_ALIVE_MS` from the start of each test, so a test can await the
   * unreferenced timers.
   */
  timers?: boolean;
  /** Set `IS_REACT_ACT_ENVIRONMENT` for direct component rendering. */
  act?: boolean;
}

/** Build a jsdom window and publish it on `globalThis`. */
export function installDom(options: InstallDomOptions = {}): JSDOM {
  const {
    url = 'http://localhost/',
    body = '',
    timers = false,
    act = false,
  } = options;

  const dom = new JSDOM(`<!doctype html><html><body>${body}</body></html>`, {
    url,
  });

  const requestAnimationFrame = (callback: FrameRequestCallback): number =>
    nativeSetTimeout(() => {
      callback(0);
    }, 0) as unknown as number;
  const cancelAnimationFrame = (id: number): void => {
    clearTimeout(id as unknown as NodeJS.Timeout);
  };

  Object.defineProperties(globalThis, {
    window: { configurable: true, value: dom.window },
    document: { configurable: true, value: dom.window.document },
    navigator: { configurable: true, value: dom.window.navigator },
    location: { configurable: true, value: dom.window.location },
    history: { configurable: true, value: dom.window.history },
    Node: { configurable: true, value: dom.window.Node },
    Element: { configurable: true, value: dom.window.Element },
    HTMLElement: { configurable: true, value: dom.window.HTMLElement },
    HTMLButtonElement: {
      configurable: true,
      value: dom.window.HTMLButtonElement,
    },
    HTMLInputElement: {
      configurable: true,
      value: dom.window.HTMLInputElement,
    },
    MouseEvent: { configurable: true, value: dom.window.MouseEvent },
    Event: { configurable: true, value: dom.window.Event },
    CustomEvent: { configurable: true, value: dom.window.CustomEvent },
    MutationObserver: {
      configurable: true,
      value: dom.window.MutationObserver,
    },
    ResizeObserver: { configurable: true, value: TestResizeObserver },
    getComputedStyle: {
      configurable: true,
      value: dom.window.getComputedStyle.bind(dom.window),
    },
    requestAnimationFrame: { configurable: true, value: requestAnimationFrame },
    cancelAnimationFrame: { configurable: true, value: cancelAnimationFrame },
  });

  Object.defineProperties(dom.window, {
    matchMedia: {
      configurable: true,
      value: () => ({
        matches: false,
        addEventListener() {},
        removeEventListener() {},
      }),
    },
    scrollTo: { configurable: true, value: () => {} },
    requestAnimationFrame: { configurable: true, value: requestAnimationFrame },
    cancelAnimationFrame: { configurable: true, value: cancelAnimationFrame },
  });

  if (timers) {
    Object.defineProperties(globalThis, {
      setInterval: {
        configurable: true,
        value: (...args: Parameters<typeof setInterval>) => {
          const timer = nativeSetInterval(...args);
          timer.unref();
          return timer;
        },
      },
      setTimeout: {
        configurable: true,
        value: (...args: Parameters<typeof setTimeout>) => {
          const timer = nativeSetTimeout(...args);
          timer.unref();
          return timer;
        },
      },
    });
    // An unreferenced timer does not keep the process alive, so a test that
    // awaits one (`waitFor` polls on `setTimeout`) would be cancelled once
    // nothing else is pending. This referenced timer keeps the event loop
    // alive for at most KEEP_ALIVE_MS from installDom and from the start of
    // each test. A test that awaits a promise nothing settles is cancelled
    // when it expires instead of holding the file open.
    const keepAlive = nativeSetTimeout(() => {}, KEEP_ALIVE_MS);
    test.beforeEach(() => {
      keepAlive.refresh();
    });
    test.after(() => {
      clearTimeout(keepAlive);
    });
  }

  if (act) {
    (
      globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
    ).IS_REACT_ACT_ENVIRONMENT = true;
  }

  return dom;
}
