import assert from 'node:assert/strict';
import test from 'node:test';
import { renderToStaticMarkup } from 'react-dom/server';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';
let vite: ViteDevServer;
let MessageText: typeof import('../src/chat/message-text').MessageText;
let safeChatLink: typeof import('../src/chat/message-text').safeChatLink;
let relativeMessageTime: typeof import('../src/chat/presentation').relativeMessageTime;
let ToastProvider: typeof import('../kit/toasts').ToastProvider;
let ToastController: typeof import('../kit/toasts').ToastController;
test.before(async () => {
  vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
  ({ MessageText, safeChatLink } = await vite.ssrLoadModule(
    '/src/chat/message-text.tsx',
  ));
  ({ relativeMessageTime } = await vite.ssrLoadModule(
    '/src/chat/presentation.ts',
  ));
  ({ ToastProvider, ToastController } =
    await vite.ssrLoadModule('/kit/toasts.tsx'));
});
test.after(async () => {
  await vite.close();
});
const actions = {
  copyText: async () => ({ ok: true as const }),
  openChatLink: async () => ({ ok: true as const }),
};
const messageMarkup = (text: string): string =>
  renderToStaticMarkup(
    createElement(ToastProvider, {
      controller: new ToastController(),
      children: createElement(MessageText, { text, actions }),
    }),
  );
test('Basic source renders safe bounded Markdown without HTML or image fetching', () => {
  const text =
    '**bold** *em* `code`\n- first\n- second\n> quote\n```\n<x>\n```\n<img src="https://evil"> ![image](https://example.com/a) [bad](javascript:alert)';
  const html = messageMarkup(text);
  assert.match(html, /<strong>bold<\/strong>/);
  assert.match(html, /<ul>/);
  assert.match(html, /<blockquote>/);
  assert.match(html, /&lt;x&gt;/);
  assert.doesNotMatch(html, /<img|href="javascript/);
  assert.match(html, /Copy code/);
  assert.doesNotMatch(html, /Copy message/);
});
test('message timestamps are relative to the current time', () => {
  const now = Date.UTC(2026, 8, 18, 12);
  assert.equal(relativeMessageTime(String(now), now), 'just now');
  assert.equal(relativeMessageTime(String(now - 120_000), now), '2 min');
  assert.equal(relativeMessageTime(String(now - 60_000), now), '1 min');
  assert.equal(relativeMessageTime(String(now - 7_200_000), now), '2 hours');
  assert.equal(relativeMessageTime(String(now - 3_600_000), now), '1 hour');
  assert.equal(relativeMessageTime(String(now - 86_400_000), now), '1 day');
  assert.equal(relativeMessageTime(String(now - 259_200_000), now), '3 days');
  // Only a clock disagreement puts a message ahead of now; it is not an age.
  assert.equal(relativeMessageTime(String(now + 300_000), now), 'in 5 min');
});

test('unsafe and disguised links remain inert', () => {
  for (const value of [
    'javascript:alert(1)',
    'java\nscript:alert(1)',
    'javascript%3Aalert',
    'data:text/html,x',
    'file:///tmp/x',
    ' https://a',
    'https://a\\b',
    'https://user@a',
  ])
    assert.equal(safeChatLink(value), null);
  assert.equal(safeChatLink('https://example.com/a'), 'https://example.com/a');
});
test('oversized line sets fall back to literal source', () => {
  const text = '**x**\n'.repeat(513);
  const html = messageMarkup(text);
  assert.doesNotMatch(html, /<strong>/);
  assert.match(html, /\*\*x\*\*/);
});

test('notification visibility requires focused intersection, not merely a loaded message', async () => {
  const { installDom } = await import('./lib/dom-harness');
  installDom({
    url: 'http://localhost/',
    body: '<div data-chat-store="s" data-chat-channel="a"><article data-message="m"></article></div>',
  });
  const { messageVisible } = await import('../src/chat/visibility');
  Object.defineProperty(document, 'hasFocus', {
    value: () => true,
    configurable: true,
  });
  const clip = document.querySelector<HTMLElement>('[data-chat-channel]')!;
  const item = document.querySelector<HTMLElement>('[data-message]')!;
  clip.getBoundingClientRect = () =>
    ({
      top: 0,
      bottom: 100,
      left: 0,
      right: 100,
      width: 100,
      height: 100,
    }) as DOMRect;
  item.getBoundingClientRect = () =>
    ({
      top: 101,
      bottom: 120,
      left: 0,
      right: 50,
      width: 50,
      height: 19,
    }) as DOMRect;
  assert.equal(messageVisible('s', 'a', 'm'), false);
  item.getBoundingClientRect = () =>
    ({
      top: 10,
      bottom: 30,
      left: 0,
      right: 50,
      width: 50,
      height: 20,
    }) as DOMRect;
  assert.equal(messageVisible('s', 'a', 'm'), true);
  assert.equal(messageVisible('other', 'a', 'm'), false);
  Object.defineProperty(document, 'hasFocus', {
    value: () => false,
    configurable: true,
  });
  assert.equal(messageVisible('s', 'a', 'm'), false);
});
