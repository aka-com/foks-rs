/**
 * Verifies screen-level error isolation, retry behavior, and boundary reset
 * when navigation changes the current location.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, useState } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import { installDom } from './lib/dom-harness';
import type { Location } from '../src/location';
import { sameLocation } from '../src/navigation/routes';

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

test('the boundary key identifies screens and page identity includes route fields', () => {
  assert.equal(
    boundary.screenBoundaryKey({ kind: 'store', ref: 'acct:personal' }),
    'store:acct:personal:',
  );
  assert.equal(
    boundary.screenBoundaryKey({ kind: 'settings', section: 'servers' }),
    'settings::servers',
  );
  assert.equal(boundary.screenBoundaryKey({ kind: 'all' }), 'all::');
  // The fields `sameLocation` distinguishes each make a different page, on
  // the same screen: the key is unchanged, the identity is not.
  const pairs: [Location, Location][] = [
    [
      { kind: 'chat', ref: 'team:eng', channel: 'a' },
      { kind: 'chat', ref: 'team:eng', channel: 'b' },
    ],
    // The Chat tab keeps its team column across a team switch: the router
    // keys the tab on its kind and the conversation on the team.
    [
      { kind: 'chat', ref: 'team:eng', channel: 'a' },
      { kind: 'chat', ref: 'team:ops', channel: 'a' },
    ],
    [{ kind: 'chat' }, { kind: 'chat', ref: 'team:eng' }],
    [
      { kind: 'group-settings', ref: 'team:eng', tab: 'people' },
      { kind: 'group-settings', ref: 'team:eng', tab: 'settings' },
    ],
    [
      { kind: 'devices', section: 'keys', store: 'acct:a', device: 'd1' },
      { kind: 'devices', section: 'keys', store: 'acct:a', device: 'd2' },
    ],
    [
      { kind: 'settings', section: 'servers', profile: 'acme' },
      { kind: 'settings', section: 'servers', profile: 'personal' },
    ],
    [
      { kind: 'people', store: 'acct:a' },
      { kind: 'people', store: 'acct:b' },
    ],
    [
      { kind: 'teams', store: 'acct:a' },
      { kind: 'teams', store: 'acct:b' },
    ],
  ];
  for (const [left, right] of pairs) {
    assert.equal(sameLocation(left, right), false);
    assert.equal(
      boundary.screenBoundaryKey(left),
      boundary.screenBoundaryKey(right),
    );
    assert.notEqual(
      boundary.screenIdentity(left),
      boundary.screenIdentity(right),
    );
  }
});

test('moving to another channel of the same team clears a failure without remounting a healthy screen', () => {
  let setChannel!: (channel: string) => void;
  let mounts = 0;
  function Page({ channel }: { channel: string }) {
    const [mounted] = useState(() => ++mounts);
    if (channel === 'broken') throw new Error('Channel broken');
    return createElement('p', null, `Channel ${channel} (${mounted})`);
  }
  function Shell() {
    const [channel, update] = useState('broken');
    setChannel = update;
    const location: Location = { kind: 'chat', ref: 'team:eng', channel };
    return createElement(boundary.ScreenErrorBoundary, {
      key: boundary.screenBoundaryKey(location),
      identity: boundary.screenIdentity(location),
      children: createElement(Page, { channel }),
    });
  }
  const { value: rendered } = quietly(() => ui.render(createElement(Shell)));
  assert.ok(rendered.getByRole('alert'));
  // The failure clears for the next channel, which mounts.
  ui.act(() => setChannel('fine'));
  assert.ok(rendered.getByText(/Channel fine/));
  assert.equal(rendered.queryByRole('alert'), null);
  const drawn = mounts;
  // A further channel of the same team keeps the healthy screen mounted.
  ui.act(() => setChannel('other'));
  assert.ok(rendered.getByText(`Channel other (${drawn})`));
  assert.equal(mounts, drawn);
});

test('opening Chat with no team and switching teams keeps the healthy tab mounted', () => {
  let setRef!: (ref: string | undefined) => void;
  let mounts = 0;
  function Tab({ team }: { team: string | undefined }) {
    const [mounted] = useState(() => ++mounts);
    if (team === 'broken') throw new Error('Team broken');
    return createElement('p', null, `Team ${team ?? 'none'} (${mounted})`);
  }
  function Shell() {
    const [ref, update] = useState<string | undefined>('broken');
    setRef = update;
    const location: Location = ref ? { kind: 'chat', ref } : { kind: 'chat' };
    return createElement(boundary.ScreenErrorBoundary, {
      key: boundary.screenBoundaryKey(location),
      identity: boundary.screenIdentity(location),
      children: createElement(Tab, { team: ref }),
    });
  }
  const { value: rendered } = quietly(() => ui.render(createElement(Shell)));
  assert.ok(rendered.getByRole('alert'));
  // The failure clears for the next team, which mounts the tab.
  ui.act(() => setRef(undefined));
  assert.ok(rendered.getByText(/Team none/));
  assert.equal(rendered.queryByRole('alert'), null);
  const drawn = mounts;
  // The tab resolves a team for the bare location and navigates to it, then
  // the user switches teams: the tab stays mounted through both.
  ui.act(() => setRef('team:eng'));
  assert.ok(rendered.getByText(`Team team:eng (${drawn})`));
  ui.act(() => setRef('team:ops'));
  assert.ok(rendered.getByText(`Team team:ops (${drawn})`));
  assert.equal(mounts, drawn);
});
