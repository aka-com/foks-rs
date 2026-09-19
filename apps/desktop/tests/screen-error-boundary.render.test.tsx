/**
 * Verifies screen-level error isolation, retry behavior, and boundary reset
 * when navigation changes the current location.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, useState } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import { installDom } from './lib/dom-harness';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div>',
  timers: true,
});

let ui: typeof import('@testing-library/react');
let vite: ViteDevServer;
let boundary: typeof import('../src/app/screen-error-boundary');
test.before(async () => {
  ui = await import('@testing-library/react');
  vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
  boundary = (await vite.ssrLoadModule(
    '/src/app/screen-error-boundary.tsx',
  )) as typeof import('../src/app/screen-error-boundary');
});
test.afterEach(() => ui.cleanup());
test.after(async () => vite.close());

/** Captures React's console report for a caught render error. */
function quietly<T>(run: () => T): { value: T; reported: unknown[][] } {
  const reported: unknown[][] = [];
  const original = console.error;
  console.error = (...args: unknown[]) => {
    reported.push(args);
  };
  try {
    return { value: run(), reported };
  } finally {
    console.error = original;
  }
}

test('a screen render error preserves shell navigation and Reload page retries the screen', () => {
  let throwing = true;
  function Page() {
    if (throwing) throw new Error('The item list could not be sorted.');
    return createElement('p', null, 'The page');
  }
  const { value: rendered, reported } = quietly(() =>
    ui.render(
      createElement(
        'div',
        null,
        createElement('nav', null, 'Rail'),
        createElement(boundary.ScreenErrorBoundary, null, createElement(Page)),
      ),
    ),
  );
  assert.ok(rendered.getByText('Rail'));
  const alert = rendered.getByRole('alert');
  assert.match(alert.textContent ?? '', /This page could not be shown/);
  assert.match(alert.textContent ?? '', /could not be sorted/);
  assert.equal(rendered.queryByText('The page'), null);
  // The original render error remains available in the console.
  assert.ok(
    reported.some((args) =>
      args.some(
        (arg) =>
          arg instanceof Error && /could not be sorted/.test(arg.message),
      ),
    ),
  );

  throwing = false;
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Reload page' }));
  assert.ok(rendered.getByText('The page'));
  assert.equal(rendered.queryByRole('alert'), null);
});

test('changing location resets the screen error boundary', () => {
  function Page({ name }: { name: string }) {
    if (name === 'broken') throw new Error('Broken page.');
    return createElement('p', null, `Page ${name}`);
  }
  let setName!: (name: string) => void;
  function Shell() {
    const [name, update] = useState('broken');
    setName = update;
    return createElement(boundary.ScreenErrorBoundary, {
      key: boundary.screenBoundaryKey({ kind: 'store', ref: name }),
      children: createElement(Page, { name }),
    });
  }
  const { value: rendered } = quietly(() => ui.render(createElement(Shell)));
  assert.ok(rendered.getByRole('alert'));
  ui.act(() => setName('fine'));
  assert.ok(rendered.getByText('Page fine'));
  assert.equal(rendered.queryByRole('alert'), null);
});

test('the key reads the fields that identify a page and nothing else', () => {
  assert.equal(
    boundary.screenBoundaryKey({ kind: 'store', ref: 'acct:personal' }),
    'store:acct:personal:',
  );
  assert.equal(
    boundary.screenBoundaryKey({ kind: 'settings', section: 'servers' }),
    'settings::servers',
  );
  assert.equal(boundary.screenBoundaryKey({ kind: 'all' }), 'all::');
});
