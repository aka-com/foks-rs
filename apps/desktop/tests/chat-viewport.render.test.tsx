import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import { installDom } from './lib/dom-harness';
import { layOutLists } from './lib/list-layout';
import type { ChatMessage } from '../src/chat-contract';
import type { ListPosition } from '../src/chat/history-cache';
import type { useChatViewport as ViewportHook } from '../src/chat/use-chat-viewport';

installDom({ act: true, timers: true });
let ui: typeof import('@testing-library/react');
let vite: ViteDevServer;
let useChatViewport: typeof ViewportHook;
let restoreLayout: () => void;
test.before(async () => {
  ui = await import('@testing-library/react');
  vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
  ({ useChatViewport } = await vite.ssrLoadModule(
    '/src/chat/use-chat-viewport.ts',
  ));
  restoreLayout = layOutLists();
});
test.afterEach(() => ui.cleanup());
test.after(async () => {
  restoreLayout();
  await vite.close();
});

const id = (sequence: number) => sequence.toString(16).padStart(32, '0');
/** Messages with sequences `from` through `to`. */
function rows(from: number, to: number): ChatMessage[] {
  return Array.from({ length: to - from + 1 }, (_, index) => ({
    id: id(from + index),
    sequence: String(from + index),
    sender: null,
    send_time: '1',
    insert_time: '1',
    content: { kind: 'text', text: `Message ${from + index}` },
  }));
}

let viewport!: ReturnType<typeof ViewportHook>;
function List({
  messages,
  resume = null,
}: {
  messages: ChatMessage[];
  resume?: ListPosition | null;
}) {
  viewport = useChatViewport(messages, '', undefined, resume);
  return createElement(
    'div',
    {
      className: 'chat-messages',
      ref: viewport.scroller,
      onScroll: viewport.onScroll,
    },
    messages.map((m) =>
      createElement('div', {
        key: m.id,
        'data-message': m.id,
        'data-sequence': m.sequence,
      }),
    ),
  );
}

/** Mounts the list over `messages` and scrolls it to `top`. */
function mount(messages: ChatMessage[], top: number) {
  const view = ui.render(createElement(List, { messages }));
  const scroller = view.container.querySelector<HTMLElement>('.chat-messages')!;
  scroller.scrollTop = top;
  ui.fireEvent.scroll(scroller);
  return {
    scroller,
    show: (next: ChatMessage[]) =>
      view.rerender(createElement(List, { messages: next })),
  };
}

test('an older page that pushes out the newest rows keeps the first row on screen in place', () => {
  // Row 103 is first on screen, its top 30px above the list's.
  const list = mount(rows(101, 150), 230);
  ui.act(() => viewport.capture('older'));
  // Fifty older rows land above it and ten of the newest rows leave. The
  // list's height grows by 4,000px, not 5,000px, so the height alone would
  // place the reader a thousand pixels off.
  list.show(rows(51, 140));
  assert.equal(list.scroller.scrollTop, 5_230);
  assert.deepEqual(viewport.position(), { message: id(103), offset: -30 });
});

test('rows that leave the oldest end above the reader leave the rows on screen in place', () => {
  const list = mount(rows(101, 150), 1_230);
  ui.act(() => viewport.capture('incremental'));
  // One new row arrives and ten of the oldest rows leave.
  list.show(rows(111, 151));
  assert.equal(list.scroller.scrollTop, 230);
  assert.deepEqual(viewport.position(), { message: id(113), offset: -30 });
});

test('a newer page keeps the row on screen when it extends the window, and starts at the top when it replaces it', () => {
  const list = mount(rows(101, 150), 2_530);
  ui.act(() => viewport.capture('newer'));
  list.show(rows(121, 170));
  assert.equal(list.scroller.scrollTop, 530);
  assert.deepEqual(viewport.position(), { message: id(126), offset: -30 });
  ui.act(() => viewport.capture('newer'));
  list.show(rows(300, 349));
  assert.equal(list.scroller.scrollTop, 0);
});

test('a list opens where the reader left it, and at the bottom without that row', () => {
  const resumed = ui.render(
    createElement(List, {
      messages: rows(101, 150),
      resume: { message: id(120), offset: -30 },
    }),
  );
  const scroller =
    resumed.container.querySelector<HTMLElement>('.chat-messages')!;
  assert.equal(scroller.scrollTop, 1_930);
  assert.equal(viewport.atBottom, false);
  assert.deepEqual(viewport.position(), { message: id(120), offset: -30 });
  ui.cleanup();
  const missing = ui.render(
    createElement(List, {
      messages: rows(101, 150),
      resume: { message: id(20), offset: -30 },
    }),
  );
  assert.equal(
    missing.container.querySelector<HTMLElement>('.chat-messages')!.scrollTop,
    5_000,
  );
  assert.equal(viewport.atBottom, true);
  // At the bottom there is no position to keep: the list opens there anyway.
  assert.equal(viewport.position(), null);
});
