/**
 * Configures the JSDOM test environment for component render tests.
 *
 * Initializes global browser DOM APIs and mock timers prior to module evaluation.
 * Production builds and tests run without global Tauri IPC objects.
 */

import { JSDOM } from 'jsdom';

export const nativeSetInterval = globalThis.setInterval;
export const nativeSetTimeout = globalThis.setTimeout;

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
  /** Unref `setInterval`/`setTimeout` so a stray timer cannot hang the run. */
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
  }

  if (act) {
    (
      globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
    ).IS_REACT_ACT_ENVIRONMENT = true;
  }

  return dom;
}
