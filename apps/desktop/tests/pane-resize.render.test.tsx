/**
 * The shared pane separator, run against every pane in PANE_SIZES: the rail
 * (bounded by `.window`, separator portalled into `.app`), the Files folder
 * tree and the Chat column (each bounded by its own frame).
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { installDom } from './lib/dom-harness';

const dom = installDom({ url: 'http://localhost/', act: true });
let ui: typeof import('@testing-library/react');
let panes: typeof import('../src/components/use-pane-resize');
let frameWidth = 1100;
let resized: (() => void) | undefined;
// The original method is always invoked with .call or restored to its prototype.
// eslint-disable-next-line @typescript-eslint/unbound-method
const originalRect = HTMLElement.prototype.getBoundingClientRect;
const originalObserver = globalThis.ResizeObserver;

type PaneName = 'rail' | 'files' | 'chat';

/** Expected widths, independent of the hook's own arithmetic. */
const EXPECT: Record<
  PaneName,
  {
    key: string;
    cssVariable: string;
    name: string;
    default: number;
    compact: number;
    min: number;
    /** Maximum in a 1100px frame. */
    max: number;
    /** A stored width below `max` and above the 600px-frame maximum. */
    wide: number;
    /** Maximum in a 600px frame. */
    narrowMax: number;
    /** [min, max] in a 300px frame. */
    tiny: [number, number];
  }
> = {
  rail: {
    key: 'sidebarWidth',
    cssVariable: '--side-w-open',
    name: 'Resize sidebar',
    default: 150,
    compact: 150,
    min: 150,
    max: 240,
    wide: 220,
    narrowMax: 180,
    tiny: [150, 150],
  },
  files: {
    key: 'filesSidebarWidth',
    cssVariable: '--files-tree-w',
    name: 'Resize Files sidebar',
    default: 232,
    compact: 220,
    min: 180,
    max: 420,
    wide: 400,
    narrowMax: 280,
    tiny: [135, 135],
  },
  chat: {
    key: 'chatSidebarWidth',
    cssVariable: '--chat-inbox-w',
    name: 'Resize Chat sidebar',
    default: 264,
    compact: 220,
    min: 180,
    max: 420,
    wide: 400,
    narrowMax: 280,
    tiny: [135, 135],
  },
};
const NAMES = Object.keys(EXPECT) as PaneName[];

test.before(async () => {
  ui = await import('@testing-library/react');
  panes = await import('../src/components/use-pane-resize');
  HTMLElement.prototype.getBoundingClientRect = function () {
    return this.classList.contains('pane-frame')
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
  dom.window.close();
});

/** Mirrors each consumer's markup: the rail in `.window > .app`, the others in their frame. */
function Harness({ pane, enabled }: { pane: PaneName; enabled: boolean }) {
  const sidebar = panes.usePaneResize(panes.PANE_SIZES[pane], enabled);
  if (pane === 'rail')
    return createElement(
      'div',
      { className: 'window pane-frame' },
      createElement(
        'div',
        { className: 'app' },
        createElement(
          'nav',
          { ref: sidebar.ref, id: sidebar.id, className: 'side rail' },
          sidebar.handle,
        ),
        createElement('main', null, 'Content'),
      ),
    );
  return createElement(
    'section',
    { ref: sidebar.ref, style: sidebar.style, className: 'pane-frame' },
    sidebar.handle,
    createElement('aside', { id: sidebar.id }, 'Sidebar'),
    createElement('main', null, 'Content'),
  );
}
function mount(pane: PaneName, enabled = true) {
  const view = ui.render(createElement(Harness, { pane, enabled }));
  const handle = separator(pane);
  return { view, handle };
}
function separator(pane: PaneName) {
  const handle = ui.screen.getByRole('separator', { name: EXPECT[pane].name });
  handle.setPointerCapture = () => {};
  handle.releasePointerCapture = () => {};
  return handle;
}
function width(handle: HTMLElement) {
  return Number(handle.getAttribute('aria-valuenow'));
}
function cssWidth(pane: PaneName) {
  return document
    .querySelector<HTMLElement>('.pane-frame')
    ?.style.getPropertyValue(EXPECT[pane].cssVariable);
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
function resizeFrame(next: number) {
  ui.act(() => {
    frameWidth = next;
    resized?.();
  });
}

for (const pane of NAMES) {
  const expect = EXPECT[pane];

  test(`${pane}: keyboard resizing persists under its own key and restores on remount`, () => {
    for (const other of NAMES)
      if (other !== pane) window.localStorage.setItem(EXPECT[other].key, '200');
    const { view, handle } = mount(pane);
    assert.equal(width(handle), expect.default);
    assert.equal(handle.getAttribute('aria-valuemin'), String(expect.min));
    assert.equal(handle.getAttribute('aria-valuemax'), String(expect.max));
    assert.equal(
      handle.getAttribute('aria-valuetext'),
      `${expect.default} pixels`,
    );
    assert.ok(document.getElementById(handle.getAttribute('aria-controls')!));
    ui.fireEvent.keyDown(handle, { key: 'ArrowRight' });
    assert.equal(width(handle), expect.default + 8);
    assert.equal(cssWidth(pane), `${expect.default + 8}px`);
    assert.equal(
      window.localStorage.getItem(expect.key),
      String(expect.default + 8),
    );
    for (const other of NAMES)
      if (other !== pane)
        assert.equal(window.localStorage.getItem(EXPECT[other].key), '200');
    view.unmount();
    const next = mount(pane);
    assert.equal(width(next.handle), expect.default + 8);
    ui.fireEvent.keyDown(next.handle, { key: 'Home' });
    assert.equal(width(next.handle), expect.min);
    ui.fireEvent.keyDown(next.handle, { key: 'ArrowLeft' });
    assert.equal(width(next.handle), expect.min);
    ui.fireEvent.keyDown(next.handle, { key: 'End' });
    assert.equal(width(next.handle), expect.max);
    ui.fireEvent.keyDown(next.handle, { key: 'ArrowRight' });
    assert.equal(width(next.handle), expect.max);
  });

  test(`${pane}: a drag commits only on release, and Escape, cancel and lost capture revert`, () => {
    const { handle } = mount(pane);
    const start = expect.default;
    pointer(handle, 'pointerdown', start);
    pointer(handle, 'pointermove', start + 40, 2);
    assert.equal(width(handle), start, 'another pointer cannot move the pane');
    pointer(handle, 'pointermove', start + 40);
    assert.equal(width(handle), start + 40);
    assert.equal(window.localStorage.getItem(expect.key), null);
    assert.equal(document.body.style.userSelect, 'none');
    assert.match(handle.className, /\bis-resizing\b/);
    pointer(handle, 'pointerup', start + 40);
    const kept = start + 40;
    assert.equal(window.localStorage.getItem(expect.key), String(kept));
    assert.equal(document.body.style.userSelect, '');
    pointer(handle, 'pointerdown', kept);
    pointer(handle, 'pointermove', kept + 30);
    assert.equal(width(handle), kept + 30);
    ui.fireEvent.keyDown(handle, { key: 'Escape' });
    assert.equal(width(handle), kept);
    assert.equal(cssWidth(pane), `${kept}px`);
    assert.equal(window.localStorage.getItem(expect.key), String(kept));
    pointer(handle, 'pointerdown', kept);
    pointer(handle, 'pointermove', kept - 30);
    pointer(handle, 'pointercancel', kept - 30);
    assert.equal(width(handle), kept);
    assert.equal(document.body.style.cursor, '');
    pointer(handle, 'pointerdown', kept);
    pointer(handle, 'pointermove', kept + 20);
    pointer(handle, 'lostpointercapture', kept + 20);
    assert.equal(width(handle), kept);
    assert.equal(window.localStorage.getItem(expect.key), String(kept));
  });

  test(`${pane}: a narrow frame bounds the width and aria-valuemax but keeps the preference; reset restores the responsive default`, () => {
    window.localStorage.setItem(expect.key, String(expect.wide));
    const { handle } = mount(pane);
    assert.equal(width(handle), expect.wide);
    resizeFrame(600);
    assert.equal(width(handle), expect.narrowMax);
    assert.equal(
      handle.getAttribute('aria-valuemax'),
      String(expect.narrowMax),
    );
    assert.equal(cssWidth(pane), `${expect.narrowMax}px`);
    pointer(handle, 'pointerdown', expect.narrowMax);
    pointer(handle, 'pointerup', expect.narrowMax);
    // Clicking the separator must not replace a wider saved preference with
    // the temporary width imposed by a narrow frame.
    assert.equal(window.localStorage.getItem(expect.key), String(expect.wide));
    resizeFrame(1100);
    assert.equal(width(handle), expect.wide);
    ui.fireEvent.doubleClick(handle);
    assert.equal(width(handle), expect.default);
    assert.equal(window.localStorage.getItem(expect.key), null);
    resizeFrame(700);
    assert.equal(width(handle), expect.compact);
    resizeFrame(300);
    assert.deepEqual(
      [
        Number(handle.getAttribute('aria-valuemin')),
        Number(handle.getAttribute('aria-valuemax')),
      ],
      expect.tiny,
    );
  });

  test(`${pane}: invalid saved preferences fall back and out-of-range values are bounded`, () => {
    for (const [stored, expected] of [
      ['garbage', expect.default],
      ['Infinity', expect.default],
      ['-10', expect.default],
      ['9999', expect.max],
      ['1', expect.min],
    ] as const) {
      window.localStorage.setItem(expect.key, stored);
      const { view, handle } = mount(pane);
      assert.equal(width(handle), expected);
      view.unmount();
    }
  });

  test(`${pane}: unmounting during a drag restores body styles and disconnects observation`, () => {
    const { view, handle } = mount(pane);
    pointer(handle, 'pointerdown', expect.default);
    pointer(handle, 'pointermove', expect.default + 30);
    view.unmount();
    assert.equal(document.body.style.cursor, '');
    assert.equal(document.body.style.userSelect, '');
    assert.equal(resized, undefined);
    assert.equal(window.localStorage.getItem(expect.key), null);
  });

  test(`${pane}: disabling removes the separator and cancels a drag in progress`, () => {
    const { view, handle } = mount(pane);
    pointer(handle, 'pointerdown', expect.default);
    pointer(handle, 'pointermove', expect.default + 30);
    assert.equal(cssWidth(pane), `${expect.default + 30}px`);
    view.rerender(createElement(Harness, { pane, enabled: false }));
    assert.equal(
      ui.screen.queryByRole('separator', { name: expect.name }),
      null,
    );
    assert.equal(document.body.style.cursor, '');
    assert.equal(cssWidth(pane), `${expect.default}px`);
    assert.equal(window.localStorage.getItem(expect.key), null);
    view.rerender(createElement(Harness, { pane, enabled: true }));
    assert.equal(width(separator(pane)), expect.default);
  });

  test(`${pane}: storage failure leaves resizing usable`, () => {
    const storage = Object.getPrototypeOf(window.localStorage) as Storage;
    const descriptors = Object.getOwnPropertyDescriptors(storage);
    try {
      storage.getItem =
        storage.setItem =
        storage.removeItem =
          () => {
            throw new Error('unavailable');
          };
      const { handle } = mount(pane);
      assert.equal(width(handle), expect.default);
      ui.fireEvent.keyDown(handle, { key: 'ArrowRight' });
      assert.equal(width(handle), expect.default + 8);
      ui.fireEvent.doubleClick(handle);
      assert.equal(width(handle), expect.default);
    } finally {
      Object.defineProperties(storage, descriptors);
    }
  });
}

test('the rail separator renders into .app and sets its width on .window', () => {
  const { handle } = mount('rail');
  assert.equal(handle.parentElement?.className, 'app');
  assert.equal(
    handle.getAttribute('aria-controls'),
    document.querySelector('nav.rail')?.id,
  );
  assert.equal(cssWidth('rail'), '150px');
  assert.equal(document.querySelector('nav.rail')?.getAttribute('style'), null);
});
