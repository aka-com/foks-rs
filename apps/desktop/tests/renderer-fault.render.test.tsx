/**
 * Renderer faults: a render error in a panel closes that panel and leaves the
 * rail and screen, a render error elsewhere replaces the window with the fault
 * screen inside the exit guard, and each fault is recorded under its
 * boundary's static label, never with its message, so Copy diagnostics
 * reports it.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import type { Bridge, ReadItemResponse } from '../src/bridge';
import type { TimingEvent } from '../src/diagnostics/log';
import type { LocationStore } from '../src/location';
import { installDom } from './lib/dom-harness';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
});

let ui: typeof import('@testing-library/react');
let vite: ViteDevServer;
let faults: typeof import('../src/diagnostics/renderer-fault');
let logs: typeof import('../src/diagnostics/log');
let boundaries: typeof import('../src/app/fault-boundaries');
test.before(async () => {
  ui = await import('@testing-library/react');
  vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
  faults = (await vite.ssrLoadModule(
    '/src/diagnostics/renderer-fault.ts',
  )) as typeof import('../src/diagnostics/renderer-fault');
  logs = (await vite.ssrLoadModule(
    '/src/diagnostics/log.ts',
  )) as typeof import('../src/diagnostics/log');
  boundaries = (await vite.ssrLoadModule(
    '/src/app/fault-boundaries.tsx',
  )) as typeof import('../src/app/fault-boundaries');
});
test.afterEach(() => ui.cleanup());
test.after(async () => vite.close());

/** Runs `body` with React's console reports captured instead of printed. */
async function quietly<T>(
  body: () => Promise<T>,
): Promise<{ value: T; reported: unknown[][] }> {
  const reported: unknown[][] = [];
  const original = console.error;
  console.error = (...args: unknown[]) => {
    reported.push(args);
  };
  try {
    return { value: await body(), reported };
  } finally {
    console.error = original;
  }
}

/** The shared log's fault events recorded since `since`. */
function faultsSince(since: number): TimingEvent[] {
  return logs.diagnosticLog
    .events()
    .slice(since)
    .filter((event) => event.name === faults.RENDERER_FAULT);
}

test('a fault code names only a built-in error type', () => {
  assert.equal(faults.faultCode(new TypeError('x')), 'TypeError');
  assert.equal(faults.faultCode(new RangeError('x')), 'RangeError');
  assert.equal(faults.faultCode(new Error('x')), 'Error');
  // A subclass's name is mangled in a minified build, so it is not reported.
  class ParseFailure extends Error {}
  assert.equal(faults.faultCode(new ParseFailure('x')), undefined);
  assert.equal(faults.faultCode('a thrown string'), undefined);
  assert.equal(faults.faultCode({ code: 'busy', message: 'x' }), undefined);
  assert.equal(faults.faultCode(null), undefined);
});

test('a fault is recorded with its label and code, never its message, and capped per label', () => {
  const log = new logs.DiagnosticLog();
  const event = faults.recordRendererFault(
    'details',
    new TypeError('secret-looking text'),
    log,
  );
  assert.deepEqual(
    { ...event, at: 0 },
    {
      at: 0,
      layer: 'renderer',
      name: 'renderer.fault',
      outcome: 'error',
      code: 'TypeError',
      attrs: { boundary: 'details' },
    },
  );
  assert.doesNotMatch(JSON.stringify(log.events()), /secret-looking/);
  for (let index = 0; index < faults.FAULTS_PER_LABEL + 10; index++)
    faults.recordRendererFault('window-error', new Error('loop'), log);
  faults.recordRendererFault('search', 'not an error', log);
  const recorded = log.events();
  assert.equal(
    recorded.filter((entry) => entry.attrs?.boundary === 'window-error').length,
    faults.FAULTS_PER_LABEL,
  );
  const search = recorded.find((entry) => entry.attrs?.boundary === 'search');
  assert.ok(search, 'a capped label does not stop another label');
  assert.equal(search.code, undefined);
});

test('the root callbacks record what no labelled boundary recorded', () => {
  const log = new logs.DiagnosticLog();
  const reported: unknown[][] = [];
  const options = faults.rendererRootOptions(log, (...args) =>
    reported.push(args),
  );
  const labelled = { faultLabel: 'screen' } as never;
  options.onCaughtError?.(new Error('x'), {
    componentStack: '',
    errorBoundary: labelled,
  });
  assert.equal(log.length, 0, 'the labelled boundary recorded it already');
  assert.equal(reported.length, 0, 'and wrote it to the console');
  options.onCaughtError?.(new RangeError('x'), {
    componentStack: '',
    errorBoundary: {} as never,
  });
  options.onUncaughtError?.(new TypeError('x'), { componentStack: '' });
  options.onRecoverableError?.(new Error('x'), { componentStack: '' });
  assert.deepEqual(
    log.events().map((event) => [event.attrs?.boundary, event.code]),
    [
      ['unlabelled', 'RangeError'],
      ['uncaught', 'TypeError'],
      ['recoverable', 'Error'],
    ],
  );
  assert.equal(reported.length, 3);
});

test('window script errors and unhandled rejections are recorded until the listeners stop', () => {
  const log = new logs.DiagnosticLog();
  const target = document.createElement('div');
  const stop = faults.listenForWindowFaults(target, log);
  target.dispatchEvent(
    Object.assign(new Event('error'), { error: new TypeError('x') }),
  );
  target.dispatchEvent(
    Object.assign(new Event('unhandledrejection'), {
      reason: { code: 'busy' },
    }),
  );
  // A notice such as the ResizeObserver loop limit carries no thrown value.
  target.dispatchEvent(Object.assign(new Event('error'), { error: null }));
  stop();
  target.dispatchEvent(
    Object.assign(new Event('error'), { error: new Error('x') }),
  );
  assert.deepEqual(
    log.events().map((event) => [event.attrs?.boundary, event.code]),
    [
      ['window-error', 'TypeError'],
      ['unhandled-rejection', undefined],
    ],
  );
});

test('Copy diagnostics lists faults by boundary and error type', async () => {
  const { formatTimings } = (await vite.ssrLoadModule(
    '/src/diagnostics/format.ts',
  )) as typeof import('../src/diagnostics/format');
  const log = new logs.DiagnosticLog(64, {
    now: () => Date.UTC(2026, 0, 1, 12, 0, 0),
    elapsed: () => 0,
  });
  faults.recordRendererFault('details', new TypeError('x'), log);
  faults.recordRendererFault('details', new TypeError('x'), log);
  faults.recordRendererFault('shell', 'thrown', log);
  const text = formatTimings(log.events(), {
    now: Date.UTC(2026, 0, 1, 12, 1, 0),
  });
  const table = text.slice(text.indexOf('--- renderer faults ---'));
  assert.match(
    table,
    /^--- renderer faults ---\nboundary\s+code\s+n\s+first\s+last\ndetails\s+TypeError\s+2\s+12:00:00\.000\s+12:00:00\.000\nshell\s+—\s+1\s/,
  );
  assert.match(text, /renderer\.fault\s+error code=TypeError boundary=details/);
});

test('Copy diagnostics lists a fault older than the timeline window', async () => {
  const { formatTimings } = (await vite.ssrLoadModule(
    '/src/diagnostics/format.ts',
  )) as typeof import('../src/diagnostics/format');
  const log = new logs.DiagnosticLog(64, {
    now: () => Date.UTC(2026, 0, 1, 12, 0, 0),
    elapsed: () => 0,
  });
  faults.recordRendererFault('shell', new TypeError('x'), log);
  const later = Date.UTC(2026, 0, 1, 14, 0, 0);
  const text = formatTimings(log.events(), { now: later });
  assert.match(
    text,
    /--- renderer faults ---\nboundary\s+code\s+n\s+first\s+last\nshell\s+TypeError\s+1\s+12:00:00\.000\s+12:00:00\.000/,
  );
  assert.match(text, /0 events/, 'the timeline still covers its window only');
  assert.match(
    boundaries.faultDiagnosticsText(log.events(), later),
    /--- renderer faults ---[\s\S]*shell\s+TypeError\s+1/,
  );
});

async function mountApp(decorate: (base: Bridge) => Partial<Bridge>) {
  const { App } = (await vite.ssrLoadModule(
    '/src/app-root.tsx',
  )) as typeof import('../src/app-root');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const { LocationStore, INITIAL_STATE } = (await vite.ssrLoadModule(
    '/src/location.ts',
  )) as typeof import('../src/location');
  const base = mockBridge(FIXTURE);
  const bridge: Bridge = { ...base, ...decorate(base) };
  const vault = 'acct:personal';
  const store: LocationStore = new LocationStore({
    ...INITIAL_STATE,
    location: { kind: 'store', ref: vault },
    folder: '/env/prod',
    selection: { store: vault, path: '/env/prod/DATABASE_URL' },
    details: true,
  });
  const rendered = ui.render(
    createElement(App, { snapshot: FIXTURE, bridge, store }),
  );
  return { rendered, store };
}

test('a render error in the details panel closes it, discards the read value and keeps the rail and screen', async () => {
  const since = logs.diagnosticLog.length;
  const { value, reported } = await quietly(async () => {
    const mounted = await mountApp(() => ({
      // A read whose value is not text: rendering it throws inside the
      // panel, after the read has resolved.
      readItem: async (request): Promise<ReadItemResponse> => ({
        store: request.storeId,
        path: request.path,
        version: request.version,
        value: { unexpected: 'hx7-Qm2' } as unknown as string,
      }),
    }));
    const panel = await ui.waitFor(() => {
      const aside = document.querySelector<HTMLElement>('aside.details');
      assert.ok(aside, 'the details panel is open');
      return aside;
    });
    ui.fireEvent.click(ui.within(panel).getByRole('button', { name: 'Show' }));
    // Compares booleans: a failed comparison would print a jsdom node.
    await ui.waitFor(() =>
      assert.equal(document.querySelector('aside.details') === null, true),
    );
    return mounted;
  });
  assert.equal(value.store.getSnapshot().details, false, 'the panel is closed');
  assert.ok(
    document.querySelector('nav.side[aria-label="Main Navigation"]'),
    'the rail remains',
  );
  assert.ok(
    document.querySelector('main.main > .topbar'),
    'the topbar remains',
  );
  assert.ok(document.querySelector('.folder-body'), 'the screen remains');
  assert.equal(
    document.querySelector('[role="alertdialog"]') === null,
    true,
    'the window fault screen is not shown',
  );
  assert.doesNotMatch(document.body.textContent ?? '', /hx7-Qm2/);
  await ui.waitFor(() =>
    assert.match(
      document.querySelector('.toasts')?.textContent ?? '',
      /The details panel could not be shown and was closed\./,
    ),
  );
  const recorded = faultsSince(since);
  assert.deepEqual(
    recorded.map((event) => [event.attrs?.boundary, event.code]),
    [['details', 'Error']],
  );
  assert.ok(
    reported.some((args) => args[0] === 'The details panel failed to render.'),
    'the error and its stack are still written to the console',
  );
});

test('a render error outside the panels shows the fault screen inside the exit guard', async () => {
  const { ExitGuard } = (await vite.ssrLoadModule(
    '/src/app/exit-guard.tsx',
  )) as typeof import('../src/app/exit-guard');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const since = logs.diagnosticLog.length;
  let quits = 0;
  const bridge: Bridge = {
    ...mockBridge(),
    quitApp: async () => {
      quits++;
    },
  };
  function Rail(): never {
    throw new TypeError('rail state was undefined');
  }
  const { value: rendered } = await quietly(async () =>
    ui.render(
      createElement(ExitGuard, {
        bridge,
        children: createElement(boundaries.WindowFaultBoundary, {
          label: 'shell',
          bridge,
          children: createElement(Rail),
        }),
      }),
    ),
  );
  const screen = rendered.getByRole('alertdialog', {
    name: 'This window could not be shown',
  });
  assert.ok(screen.closest('.exit-guard-background'), 'inside the exit guard');
  assert.equal(
    document.activeElement ===
      rendered.getByRole('button', { name: 'Reload window' }),
    true,
    'focus moves into the card',
  );
  assert.match(screen.textContent ?? '', /rail state was undefined/);
  assert.deepEqual(
    faultsSince(since).map((event) => [event.attrs?.boundary, event.code]),
    [['shell', 'TypeError']],
  );

  ui.fireEvent.click(rendered.getByRole('button', { name: 'Quit FOKS' }));
  await ui.waitFor(() => assert.equal(quits, 1));

  let copied = '';
  const clipboard = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: {
      writeText: async (text: string) => {
        copied = text;
      },
    },
  });
  try {
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'Copy diagnostics' }),
    );
    await rendered.findByRole('button', { name: 'Copied' });
  } finally {
    if (clipboard) Object.defineProperty(navigator, 'clipboard', clipboard);
    else delete (navigator as { clipboard?: unknown }).clipboard;
  }
  assert.match(copied, /renderer fault screen/);
  assert.match(copied, /--- renderer faults ---[\s\S]*shell\s+TypeError\s+1/);
  assert.doesNotMatch(copied, /rail state was undefined/);
});

test('a closed panel boundary opens again once its identity changes', async () => {
  let throwing = true;
  function Search() {
    if (throwing) throw new RangeError('x');
    return createElement('p', null, 'Search results');
  }
  let closes = 0;
  const draw = (open: boolean) =>
    createElement(
      'div',
      null,
      createElement('nav', null, 'Rail'),
      createElement(boundaries.PanelErrorBoundary, {
        label: 'search',
        identity: String(open),
        onFault: () => closes++,
        children: open ? createElement(Search) : null,
      }),
    );
  const { value: rendered } = await quietly(async () => ui.render(draw(true)));
  assert.equal(closes, 1);
  assert.ok(rendered.getByText('Rail'));
  assert.equal(rendered.queryByText('Search results') === null, true);
  throwing = false;
  rendered.rerender(draw(false));
  rendered.rerender(draw(true));
  assert.ok(rendered.getByText('Search results'));
});
