/**
 * Message text interactions: a code block's copy button and its line
 * collapse, and a bare URL that opens like any other link.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';

import { installDom, settle } from './lib/dom-harness';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
  act: true,
});

let ui: typeof import('@testing-library/react');
let MessageText: typeof import('../src/chat/message-text').MessageText;
let overlays: typeof import('../kit/overlay-primitives');
let toasts: typeof import('../kit/toasts');

test.before(async () => {
  ui = await import('@testing-library/react');
  ({ MessageText } = await import('../src/chat/message-text'));
  overlays = await import('../kit/overlay-primitives');
  toasts = await import('../kit/toasts');
});
test.afterEach(() => ui.cleanup());

function mount(text: string) {
  const opened: string[] = [];
  const copied: string[] = [];
  const shown: string[] = [];
  const controller = new toasts.ToastController();
  controller.show = (message: string) => {
    shown.push(message);
    return shown.length;
  };
  const actions = {
    copyText: async (value: string) => {
      copied.push(value);
      return { ok: true as const };
    },
    openChatLink: async (value: string) => {
      opened.push(value);
      return { ok: true as const };
    },
  };
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  ui.render(
    createElement(toasts.ToastProvider, {
      controller,
      children: createElement(overlays.OverlayProvider, {
        backgroundRef: { current: null },
        portalRoot,
        children: createElement(MessageText, { text, actions }),
      }),
    }),
  );
  return { opened, copied, shown };
}

const LOG = Array.from({ length: 30 }, (_, i) => `line ${i + 1}`).join('\n');

test('a long code block shows its first lines until expanded, and copies all of it', async () => {
  const { copied, shown } = mount(`Build output:\n\`\`\`\n${LOG}\n\`\`\``);
  const code = document.querySelector('.chat-code pre code');
  assert.ok(code);
  assert.equal(code.textContent?.split('\n').length, 12);
  const more = ui.screen.getByRole('button', { name: 'Show all 30 lines' });
  assert.equal(more.getAttribute('aria-expanded'), 'false');
  assert.equal(
    more.getAttribute('aria-controls'),
    document.querySelector('.chat-code pre')?.id,
  );
  ui.fireEvent.click(more);
  assert.equal(code.textContent, LOG);
  assert.equal(more.getAttribute('aria-expanded'), 'true');
  assert.equal(more.textContent, 'Show fewer lines');
  ui.fireEvent.click(more);
  assert.equal(code.textContent?.split('\n').length, 12);

  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Copy code' }));
  await settle();
  assert.deepEqual(copied, [LOG], 'the whole block is copied while collapsed');
  assert.deepEqual(shown, ['Code copied']);
});

test('a bare URL opens directly, without its trailing full stop', async () => {
  const { opened } = mount('Release notes: https://example.com/notes.');
  const link = ui.screen.getByRole('link');
  assert.equal(link.textContent, 'https://example.com/notes');
  ui.fireEvent.click(link);
  await settle();
  assert.deepEqual(opened, ['https://example.com/notes']);
});
