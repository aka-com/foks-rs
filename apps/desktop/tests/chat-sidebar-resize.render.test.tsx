import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import { installDom } from './lib/dom-harness';

const dom = installDom({ url: 'http://localhost/', act: true });
let ui: typeof import('@testing-library/react');
let vite: ViteDevServer;
let useResize: typeof import('../src/screens/use-chat-sidebar-resize').useChatSidebarResize;
let frameWidth = 1100;
let resized: (() => void) | undefined;
// The original method is always invoked with .call or restored to its prototype.
// eslint-disable-next-line @typescript-eslint/unbound-method
const originalRect = HTMLElement.prototype.getBoundingClientRect;
const originalObserver = globalThis.ResizeObserver;

test.before(async () => {
  ui = await import('@testing-library/react');
  vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
  ({ useChatSidebarResize: useResize } = await vite.ssrLoadModule(
    '/src/screens/use-chat-sidebar-resize.tsx',
  ));
  HTMLElement.prototype.getBoundingClientRect = function () {
    return this.classList.contains('chat-screen')
      ? { ...originalRect.call(this), width: frameWidth }
      : originalRect.call(this);
  };
  Object.defineProperty(globalThis, 'ResizeObserver', {
    configurable: true,
    value: class {
      constructor(callback: ResizeObserverCallback) {
        resized = () => callback([], this);
      }
      observe() {}
      unobserve() {}
      disconnect() {
        resized = undefined;
      }
    },
  });
});
test.afterEach(() => {
  ui.cleanup();
  window.localStorage.clear();
  frameWidth = 1100;
});
test.after(async () => {
  HTMLElement.prototype.getBoundingClientRect = originalRect;
  Object.defineProperty(globalThis, 'ResizeObserver', {
    configurable: true,
    value: originalObserver,
  });
  await vite.close();
  dom.window.close();
});

function Harness() {
  const sidebar = useResize();
  return createElement(
    'section',
    { ref: sidebar.ref, style: sidebar.style, className: 'chat-screen' },
    sidebar.handle,
    createElement('aside', { id: sidebar.id }, 'Channels'),
    createElement('main', null, 'Conversation'),
  );
}
function mount() {
  const view = ui.render(createElement(Harness));
  const handle = ui.screen.getByRole('separator', {
    name: 'Resize Chat sidebar',
  });
  handle.setPointerCapture = () => {};
  handle.releasePointerCapture = () => {};
  return { view, handle };
}
function width(handle: HTMLElement) {
  return Number(handle.getAttribute('aria-valuenow'));
}
function pointer(handle: HTMLElement, type: string, x: number, id = 1) {
  const event = new window.MouseEvent(type, {
    bubbles: true,
    clientX: x,
    button: 0,
  });
  Object.defineProperty(event, 'pointerId', { value: id });
  ui.fireEvent(handle, event);
}

test('keyboard resizing persists independently of the main sidebar and restores on remount', () => {
  window.localStorage.setItem('sidebarWidth', '200');
  const { view, handle } = mount();
  assert.equal(width(handle), 264);
  assert.ok(document.getElementById(handle.getAttribute('aria-controls')!));
  ui.fireEvent.keyDown(handle, { key: 'ArrowRight' });
  assert.equal(width(handle), 272);
  assert.equal(
    document
      .querySelector<HTMLElement>('.chat-screen')
      ?.style.getPropertyValue('--chat-inbox-w'),
    '272px',
  );
  assert.equal(window.localStorage.getItem('chatSidebarWidth'), '272');
  assert.equal(window.localStorage.getItem('sidebarWidth'), '200');
  view.unmount();
  const next = mount();
  assert.equal(width(next.handle), 272);
  ui.fireEvent.keyDown(next.handle, { key: 'Home' });
  assert.equal(width(next.handle), 180);
  ui.fireEvent.keyDown(next.handle, { key: 'End' });
  assert.equal(width(next.handle), 420);
  ui.fireEvent.keyDown(next.handle, { key: 'ArrowRight' });
  assert.equal(width(next.handle), 420);
});

test('drag commits only on release and cancelled drags restore the previous width', () => {
  const { handle } = mount();
  pointer(handle, 'pointerdown', 264);
  pointer(handle, 'pointermove', 320, 2);
  assert.equal(width(handle), 264, 'another pointer cannot move the sidebar');
  pointer(handle, 'pointermove', 320);
  assert.equal(width(handle), 320);
  assert.equal(window.localStorage.getItem('chatSidebarWidth'), null);
  assert.equal(document.body.style.userSelect, 'none');
  pointer(handle, 'pointerup', 320);
  assert.equal(window.localStorage.getItem('chatSidebarWidth'), '320');
  assert.equal(document.body.style.userSelect, '');
  pointer(handle, 'pointerdown', 320);
  pointer(handle, 'pointermove', 390);
  ui.fireEvent.keyDown(handle, { key: 'Escape' });
  assert.equal(width(handle), 320);
  assert.equal(window.localStorage.getItem('chatSidebarWidth'), '320');
  pointer(handle, 'pointerdown', 320);
  pointer(handle, 'pointermove', 200);
  pointer(handle, 'pointercancel', 200);
  assert.equal(width(handle), 320);
  assert.equal(document.body.style.cursor, '');
  pointer(handle, 'pointerdown', 320);
  pointer(handle, 'pointermove', 370);
  pointer(handle, 'lostpointercapture', 370);
  assert.equal(width(handle), 320);
});

test('narrowing the window preserves the saved preference and reset restores responsive defaults', () => {
  window.localStorage.setItem('chatSidebarWidth', '400');
  const { handle } = mount();
  ui.act(() => {
    frameWidth = 600;
    resized?.();
  });
  assert.equal(width(handle), 280);
  assert.equal(handle.getAttribute('aria-valuemax'), '280');
  pointer(handle, 'pointerdown', 280);
  pointer(handle, 'pointerup', 280);
  // Clicking the separator must not replace a wider saved preference with
  // the temporary width imposed by a narrow window.

  assert.equal(window.localStorage.getItem('chatSidebarWidth'), '400');
  ui.act(() => {
    frameWidth = 1100;
    resized?.();
  });
  assert.equal(width(handle), 400);
  ui.fireEvent.doubleClick(handle);
  assert.equal(width(handle), 264);
  assert.equal(window.localStorage.getItem('chatSidebarWidth'), null);
  ui.act(() => {
    frameWidth = 700;
    resized?.();
  });
  assert.equal(width(handle), 220);
});

test('invalid saved preferences fall back and out-of-range values are bounded', () => {
  for (const [stored, expected] of [
    ['garbage', 264],
    ['Infinity', 264],
    ['-10', 264],
    ['9999', 420],
    ['1', 180],
  ] as const) {
    window.localStorage.setItem('chatSidebarWidth', stored);
    const { view, handle } = mount();
    assert.equal(width(handle), expected);
    view.unmount();
  }
});

test('unmounting during a drag restores body styles and disconnects observation', () => {
  const { view, handle } = mount();
  pointer(handle, 'pointerdown', 264);
  pointer(handle, 'pointermove', 300);
  view.unmount();
  assert.equal(document.body.style.cursor, '');
  assert.equal(document.body.style.userSelect, '');
  assert.equal(resized, undefined);
  assert.equal(window.localStorage.getItem('chatSidebarWidth'), null);
});

test('storage failure leaves resizing usable', () => {
  const storage = Object.getPrototypeOf(window.localStorage) as Storage;
  const descriptors = Object.getOwnPropertyDescriptors(storage);
  try {
    storage.getItem =
      storage.setItem =
      storage.removeItem =
        () => {
          throw new Error('unavailable');
        };
    const { handle } = mount();
    assert.equal(width(handle), 264);
    ui.fireEvent.keyDown(handle, { key: 'ArrowLeft' });
    assert.equal(width(handle), 256);
    ui.fireEvent.doubleClick(handle);
    assert.equal(width(handle), 264);
  } finally {
    Object.defineProperties(storage, descriptors);
  }
});
