/**
 * Chat links whose label names a different host than the destination: the
 * click raises Check this link instead of opening, and each of its three
 * actions does only what it names. A link without a mismatch opens directly.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';

import { installDom, settle } from './lib/dom-harness';
import { readSource } from './lib/source';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
  act: true,
});

let ui: typeof import('@testing-library/react');
let MessageText: typeof import('../src/chat/message-text').MessageText;
let useMessageActions: typeof import('../src/chat/message-actions').useMessageActions;
let overlays: typeof import('../kit/overlay-primitives');
let toasts: typeof import('../kit/toasts');

test.before(async () => {
  ui = await import('@testing-library/react');
  ({ MessageText } = await import('../src/chat/message-text'));
  ({ useMessageActions } = await import('../src/chat/message-actions'));
  overlays = await import('../kit/overlay-primitives');
  toasts = await import('../kit/toasts');
});
test.afterEach(() => ui.cleanup());

const SPOOF =
  'Sign in at [https://bank.example/login](https://evil.example/x) or read [the notes](https://docs.example/n).';

/** The text in a row that carries Copy message, as a message row does. */
function Row({
  text,
  actions,
}: {
  text: string;
  actions: Parameters<typeof MessageText>[0]['actions'];
}) {
  const row = useMessageActions({ text }, actions);
  return createElement(
    'article',
    { onContextMenu: row.onContextMenu, onKeyDown: row.onKeyDown },
    createElement(MessageText, { text, actions }),
    row.actions,
  );
}

function mount(text = SPOOF) {
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
        children: createElement(Row, { text, actions }),
      }),
    }),
  );
  return { opened, copied, shown };
}

function link(name: RegExp): HTMLAnchorElement {
  const found = [...document.querySelectorAll<HTMLAnchorElement>('a')].find(
    (node) => name.test(node.textContent ?? ''),
  );
  assert.ok(found, `a link named ${name}`);
  return found;
}

function dialog(): HTMLElement | null {
  return document.querySelector<HTMLElement>('[role="alertdialog"]');
}

function button(label: string): HTMLButtonElement {
  const found = [
    ...document.querySelectorAll<HTMLButtonElement>(
      '[role="alertdialog"] .ft .btn',
    ),
  ].find((node) => node.textContent === label);
  assert.ok(found, `the sheet offers ${label}`);
  return found;
}

test('a mismatched link opens Check this link with Cancel focused, and Cancel opens nothing', async () => {
  const { opened } = mount();
  const spoof = link(/bank\.example/);
  assert.equal(spoof.title, 'https://evil.example/x');
  spoof.focus();
  ui.fireEvent.click(spoof);
  const sheet = dialog();
  assert.ok(sheet, 'the click raises the confirmation');
  assert.equal(opened.length, 0);
  assert.match(
    sheet.textContent ?? '',
    /The link text names bank\.example, but the link opens evil\.example\./,
  );
  assert.match(sheet.textContent ?? '', /Openshttps:\/\/evil\.example\/x/);
  const description = document.getElementById(
    sheet.getAttribute('aria-describedby') ?? '',
  );
  assert.match(
    description?.textContent ?? '',
    /names bank\.example, but the link opens evil\.example/,
    'the sheet is described by the sentence naming both hosts',
  );
  assert.equal(document.activeElement?.textContent, 'Cancel');
  ui.fireEvent.click(button('Cancel'));
  await settle();
  assert.equal(dialog(), null);
  assert.equal(opened.length, 0);
  assert.equal(document.activeElement, spoof, 'focus returns to the link');
});

test('Escape dismisses Check this link without opening and returns focus to the link', async () => {
  const { opened } = mount();
  const spoof = link(/bank\.example/);
  spoof.focus();
  ui.fireEvent.click(spoof);
  const sheet = dialog();
  assert.ok(sheet);
  ui.fireEvent.keyDown(sheet, { key: 'Escape' });
  await settle();
  assert.equal(dialog(), null);
  assert.equal(opened.length, 0);
  assert.equal(document.activeElement, spoof, 'focus returns to the link');
});

test('Open anyway opens the destination, not the label', async () => {
  const { opened } = mount();
  ui.fireEvent.click(link(/bank\.example/));
  ui.fireEvent.click(button('Open anyway'));
  await settle();
  assert.equal(dialog(), null);
  assert.deepEqual(opened, ['https://evil.example/x']);
});

test('Copy link copies the destination and opens nothing', async () => {
  const { opened, copied, shown } = mount();
  ui.fireEvent.click(link(/bank\.example/));
  ui.fireEvent.click(button('Copy link'));
  await settle();
  assert.equal(dialog(), null);
  assert.deepEqual(copied, ['https://evil.example/x']);
  assert.deepEqual(shown, ['Link copied']);
  assert.equal(opened.length, 0);
});

test('a link whose label names no host opens directly', async () => {
  const { opened } = mount();
  ui.fireEvent.click(link(/the notes/));
  await settle();
  assert.equal(dialog(), null);
  assert.deepEqual(opened, ['https://docs.example/n']);
});

test('each link names its destination host to assistive technology once', () => {
  mount();
  assert.ok(
    ui.screen.getByRole('link', {
      name: /^Link text names a different host: ?https:\/\/bank\.example\/login, opens evil\.example$/,
    }),
  );
  assert.ok(
    ui.screen.getByRole('link', { name: 'the notes, opens docs.example' }),
  );
  for (const suffix of document.querySelectorAll('.chat-link-host'))
    assert.equal(suffix.getAttribute('aria-hidden'), 'true');
});

test('a long destination host wraps instead of overflowing the message list', async () => {
  // jsdom does not lay out, so the rule is read from the stylesheet: a
  // nowrap host wider than the column would scroll its registrable name out
  // of view (bank.example.<long label>.evil.example).
  const css = await readSource('../src/screens/chat.css', import.meta.url);
  const rule = /\.chat-link-host\s*\{([^}]*)\}/.exec(css)?.[1] ?? '';
  assert.match(rule, /display: inline-block;/);
  assert.doesNotMatch(rule, /white-space/);
  assert.match(
    css,
    /\.chat-message-text\s*\{[^}]*overflow-wrap: anywhere;/,
    'the host inherits a break opportunity at any character',
  );
});

test('a context menu inside the sheet does not open the row menu', async () => {
  mount();
  ui.fireEvent.click(link(/bank\.example/));
  const sheet = dialog();
  assert.ok(sheet);
  ui.fireEvent.contextMenu(sheet.querySelector('code')!);
  await settle();
  assert.equal(
    ui.screen.queryByRole('menu', { name: 'Message actions' }),
    null,
  );
  ui.fireEvent.contextMenu(document.querySelector('.chat-message-text p')!);
  await settle();
  assert.ok(
    ui.screen.getByRole('menuitem', { name: 'Copy message' }),
    'the message text itself still opens the row menu',
  );
});
