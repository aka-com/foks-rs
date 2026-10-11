/**
 * Chat links whose label names a different host than the destination: the
 * click raises Check this link instead of opening, and each of its three
 * actions does only what it names. A link without a mismatch opens directly.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import { installDom, settle } from './lib/dom-harness';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
  act: true,
});

let ui: typeof import('@testing-library/react');
let vite: ViteDevServer;
let MessageText: typeof import('../src/chat/message-text').MessageText;
let overlays: typeof import('../kit/overlay-primitives');
let toasts: typeof import('../kit/toasts');

test.before(async () => {
  ui = await import('@testing-library/react');
  vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
  ({ MessageText } = await vite.ssrLoadModule('/src/chat/message-text.tsx'));
  overlays = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives');
  toasts = (await vite.ssrLoadModule(
    '/kit/toasts.tsx',
  )) as typeof import('../kit/toasts');
});
test.afterEach(() => ui.cleanup());
test.after(async () => vite.close());

const SPOOF =
  'Sign in at [https://bank.example/login](https://evil.example/x) or read [the notes](https://docs.example/n).';

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
        children: createElement(MessageText, { text, actions }),
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

test('a context menu inside the sheet does not open the message menu', async () => {
  mount();
  ui.fireEvent.click(link(/bank\.example/));
  const sheet = dialog();
  assert.ok(sheet);
  ui.fireEvent.contextMenu(sheet.querySelector('code')!);
  await settle();
  assert.equal(
    [...document.querySelectorAll('button')].some(
      (node) => node.textContent === 'Copy message',
    ),
    false,
  );
  ui.fireEvent.contextMenu(document.querySelector('.chat-message-text p')!);
  await settle();
  assert.ok(
    [...document.querySelectorAll('button')].some(
      (node) => node.textContent === 'Copy message',
    ),
    'the message text itself still opens the menu',
  );
});
