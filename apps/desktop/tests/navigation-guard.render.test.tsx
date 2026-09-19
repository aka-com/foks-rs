/**
 * The navigation guard as the reader meets it: a screen registering a guard
 * through the context, the confirmation a `prompt` verdict raises, and what
 * confirming, canceling and Escape each do to the navigation behind it.
 *
 * The host below is the shell's wiring in miniature — the prompter that turns
 * a verdict into the dialog, and the answer that settles the store's promise.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, useCallback, useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import { installDom } from './lib/dom-harness';
import type { GuardVerdict, LocationStore } from '../src/location';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
  act: true,
});

let ui: typeof import('@testing-library/react');
let vite: ViteDevServer;
let location: typeof import('../src/location');
let guards: typeof import('../src/navigation-guard');
let overlays: typeof import('../kit/overlay-primitives');

test.before(async () => {
  ui = await import('@testing-library/react');
  vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
  location = (await vite.ssrLoadModule(
    '/src/location.ts',
  )) as typeof import('../src/location');
  guards = (await vite.ssrLoadModule(
    '/src/navigation-guard.tsx',
  )) as typeof import('../src/navigation-guard');
  overlays = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives');
});
test.afterEach(() => ui.cleanup());
test.after(async () => vite.close());

const DISCARD: GuardVerdict = {
  verdict: 'prompt',
  title: 'Discard unsaved changes?',
  body: 'Your draft has not been saved.',
  confirm: 'Discard changes',
};

interface Pending {
  verdict: Extract<GuardVerdict, { verdict: 'prompt' }>;
  settle: (confirmed: boolean) => void;
}

/**
 * The shell's own arrangement: a screen registering `verdict` as its guard,
 * and the prompter that draws whatever the store asks about.
 */
function mount(store: LocationStore, verdict: GuardVerdict) {
  const { NavigationGuardProvider, NavigationPrompt, useNavigationGuard } =
    guards;
  const { OverlayProvider } = overlays;
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);

  function Screen(): ReactNode {
    const guard = useCallback(() => verdict, []);
    useNavigationGuard(guard);
    return createElement('button', { type: 'button' }, 'Back there');
  }

  function Host(): ReactNode {
    const [prompt, setPrompt] = useState<Pending | null>(null);
    const promptRef = useRef<Pending | null>(null);
    const settle = useCallback((confirmed: boolean) => {
      const open = promptRef.current;
      if (!open) return;
      promptRef.current = null;
      setPrompt(null);
      open.settle(confirmed);
    }, []);
    useEffect(() => {
      store.setPrompter(
        (asked) =>
          new Promise<boolean>((resolve) => {
            const next = { verdict: asked, settle: resolve };
            promptRef.current = next;
            setPrompt(next);
          }),
      );
      return () => store.setPrompter(null);
    }, []);
    return createElement(NavigationGuardProvider, {
      store,
      children: [
        createElement(Screen, { key: 'screen' }),
        prompt
          ? createElement(NavigationPrompt, {
              key: 'prompt',
              verdict: prompt.verdict,
              onConfirm: () => settle(true),
              onCancel: () => settle(false),
            })
          : null,
      ],
    });
  }

  return ui.render(
    createElement(OverlayProvider, {
      backgroundRef: { current: null },
      portalRoot,
      children: createElement(Host),
    }),
  );
}

/** The confirmation's panel, or `null` when nothing is up. */
function dialog(): HTMLElement | null {
  return document.querySelector<HTMLElement>('[role="alertdialog"]');
}

function button(label: string): HTMLButtonElement {
  const found = [...document.querySelectorAll<HTMLButtonElement>('.ft .btn')];
  const target = found.find((node) => node.textContent === label);
  assert.ok(target, `the dialog offers ${label}`);
  return target;
}

/** Runs the navigation the guard is asked about, and lets the dialog mount. */
async function leave(store: LocationStore): Promise<void> {
  await ui.act(async () => {
    store.navigate({ kind: 'files' });
    await Promise.resolve();
  });
}

test('a prompt verdict draws its own words and opens on Cancel', async () => {
  const store = new location.LocationStore();
  mount(store, DISCARD);
  await leave(store);

  const panel = dialog();
  assert.ok(panel, 'the confirmation is up');
  assert.equal(panel.querySelector('.hd h2')?.textContent, DISCARD.title);
  assert.equal(
    panel.querySelector('.sb p')?.textContent,
    'Your draft has not been saved.',
  );
  // The move is the danger button; staying is the plain one, and it has focus,
  // so the confirmation takes a deliberate second keystroke.
  const confirm = button('Discard changes');
  assert.ok(confirm.classList.contains('danger'));
  assert.equal(document.activeElement, button('Cancel'));
  // Nothing has moved while the question is open.
  assert.deepEqual(store.getSnapshot().location, { kind: 'all' });
});

test('confirming the prompt makes the navigation it asked about', async () => {
  const store = new location.LocationStore();
  mount(store, DISCARD);
  await leave(store);

  await ui.act(async () => {
    ui.fireEvent.click(button('Discard changes'));
    await Promise.resolve();
  });
  assert.equal(dialog(), null);
  assert.deepEqual(store.getSnapshot().location, { kind: 'files' });
});

test('Cancel, Escape and the backdrop keep the reader where they are', async () => {
  const store = new location.LocationStore();
  mount(store, DISCARD);

  await leave(store);
  await ui.act(async () => {
    ui.fireEvent.click(button('Cancel'));
    await Promise.resolve();
  });
  assert.equal(dialog(), null);
  assert.deepEqual(store.getSnapshot().location, { kind: 'all' });

  await leave(store);
  const escaped = dialog();
  assert.ok(escaped);
  await ui.act(async () => {
    ui.fireEvent.keyDown(escaped, { key: 'Escape' });
    await Promise.resolve();
  });
  assert.equal(dialog(), null);
  assert.deepEqual(store.getSnapshot().location, { kind: 'all' });

  // The backdrop is the dialog element itself, outside the panel it draws.
  await leave(store);
  const backdrop = dialog();
  assert.ok(backdrop);
  await ui.act(async () => {
    ui.fireEvent.mouseDown(backdrop);
    await Promise.resolve();
  });
  assert.equal(dialog(), null);
  assert.deepEqual(store.getSnapshot().location, { kind: 'all' });
});

test('a refusal raises no dialog and moves nothing', async () => {
  const store = new location.LocationStore();
  const refused: string[] = [];
  store.setRefusalHandler((reason) => refused.push(reason));
  mount(store, { verdict: 'refuse', reason: 'A key is being enrolled.' });
  await leave(store);
  assert.equal(dialog(), null);
  assert.deepEqual(refused, ['A key is being enrolled.']);
  assert.deepEqual(store.getSnapshot().location, { kind: 'all' });
});

test('a screen that unmounts takes its guard with it', async () => {
  const store = new location.LocationStore();
  const rendered = mount(store, DISCARD);
  assert.notEqual(
    store.navigationVerdict({ kind: 'navigate', location: { kind: 'files' } }),
    null,
  );
  rendered.unmount();
  assert.equal(
    store.navigationVerdict({ kind: 'navigate', location: { kind: 'files' } }),
    null,
  );
  store.navigate({ kind: 'files' });
  assert.deepEqual(store.getSnapshot().location, { kind: 'files' });
});
