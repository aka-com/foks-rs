/**
 * Verifies that blocking modal overlays display error details and provide exit
 * actions.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, useRef, useState } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import type { Bridge, MaintenanceSnapshot } from '../src/bridge';
import { installDom } from './lib/dom-harness';

const dom = installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
});

let vite: ViteDevServer;
let ui: typeof import('@testing-library/react');

test.before(async () => {
  ui = await import('@testing-library/react');
  vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
});

test.afterEach(() => {
  ui.cleanup();
  window.localStorage.clear();
  window.history.replaceState(null, '', '/');
});

test.after(async () => {
  await vite.close();
  dom.window.close();
});

async function modules() {
  const { App } = (await vite.ssrLoadModule(
    '/src/app-root.tsx',
  )) as typeof import('../src/app-root');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  return { App, FIXTURE, mockBridge };
}

function deferred<T>(): {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (error: unknown) => void;
} {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((accept, decline) => {
    resolve = accept;
    reject = decline;
  });
  return { promise, resolve, reject };
}

async function agentLostOverlay(options: {
  message?: string;
  retryAgentConnection?: Bridge['retryAgentConnection'];
  quit?: () => Promise<void>;
}) {
  const { App, FIXTURE, mockBridge } = await modules();
  let connectionLoss: string | null =
    options.message ?? 'The agent socket closed while reading the catalog.';
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    takeAgentConnectionLoss: async () => {
      const pending = connectionLoss;
      connectionLoss = null;
      return pending;
    },
    retryAgentConnection:
      options.retryAgentConnection ?? (async () => ({ state: 'ready' })),
    quitApp: options.quit ?? (async () => {}),
  };
  const rendered = ui.render(createElement(App, { bridge }));
  await ui.waitFor(() => assert.ok(document.querySelector('.stopwrap')));
  return { rendered, bridge };
}

test('agent loss automatically recovers once and manual Retry shares the active attempt', async () => {
  const { App, FIXTURE, mockBridge } = await modules();
  const base = mockBridge(FIXTURE);
  const ready = deferred<Awaited<ReturnType<Bridge['agentStatus']>>>();
  let loss = true,
    automatic = 0,
    manual = 0,
    initialized = 0,
    catalogs = 0,
    recovering = false,
    chatDuringRecovery = 0;
  const bridge: Bridge = {
    ...base,
    native: true,
    takeAgentConnectionLoss: async () => {
      if (!loss) return null;
      loss = false;
      return 'Agent endpoint closed';
    },
    autoRecoverAgent: () => {
      automatic++;
      recovering = true;
      return ready.promise;
    },
    chat: (...args) => {
      if (recovering) chatDuringRecovery++;
      return base.chat(...args);
    },
    retryAgentConnection: async () => {
      manual++;
      return { state: 'ready' };
    },
    initializeClientState: async () => {
      initialized++;
      return { state: 'ready' };
    },
    listCatalog: async () => {
      catalogs++;
      return base.listCatalog();
    },
  };
  const rendered = ui.render(createElement(App, { snapshot: FIXTURE, bridge }));
  await rendered.findByRole('alertdialog', {
    name: 'Connection to background service lost',
  });
  await ui.act(async () => {
    rendered.getByRole('button', { name: 'Retry' }).click();
  });
  assert.equal(automatic, 1);
  assert.equal(manual, 0);
  await ui.act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 20));
  });
  assert.equal(chatDuringRecovery, 0);
  await ui.act(async () => {
    recovering = false;
    ready.resolve({ state: 'ready' });
  });
  await ui.waitFor(() =>
    assert.equal(document.querySelector('.stopwrap'), null),
  );
  assert.ok(catalogs > 0);
  assert.equal(initialized, 0);
});

test('automatic recovery reporting bootstrap never initializes client state', async () => {
  const { App, FIXTURE, mockBridge } = await modules();
  let loss = true,
    initialized = 0,
    automatic = 0;
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    takeAgentConnectionLoss: async () => {
      if (!loss) return null;
      loss = false;
      return 'Agent endpoint closed';
    },
    autoRecoverAgent: async () => {
      automatic++;
      return { state: 'bootstrap', step: 'initialize-state' };
    },
    initializeClientState: async () => {
      initialized++;
      return { state: 'ready' };
    },
  };
  ui.render(createElement(App, { snapshot: FIXTURE, bridge }));
  await ui.waitFor(() => assert.equal(automatic, 1));
  await ui.act(async () => {
    await Promise.resolve();
  });
  assert.equal(initialized, 0);
});

async function agentLostWithDeferredCatalog() {
  const { App, FIXTURE, mockBridge } = await modules();
  const base = mockBridge(FIXTURE);
  const retryCatalog = deferred<Awaited<ReturnType<Bridge['listCatalog']>>>();
  const catalogEntered = deferred<void>();
  let catalogCalls = 0;
  let connectionLoss: string | null =
    'The agent socket closed while reading the catalog.';
  const bridge: Bridge = {
    ...base,
    native: true,
    takeAgentConnectionLoss: async () => {
      const pending = connectionLoss;
      connectionLoss = null;
      return pending;
    },
    retryAgentConnection: async () => ({ state: 'ready' }),
    listCatalog: () => {
      catalogCalls++;
      if (catalogCalls === 1) return base.listCatalog();
      catalogEntered.resolve(undefined);
      return retryCatalog.promise;
    },
  };
  const rendered = ui.render(createElement(App, { bridge }));
  await rendered.findByRole('alertdialog', {
    name: 'Connection to background service lost',
  });
  return { rendered, retryCatalog, catalogEntered };
}

async function startupOverlay(
  snapshot: MaintenanceSnapshot,
  overrides: Partial<Bridge> = {},
) {
  const { App, FIXTURE, mockBridge } = await modules();
  const base = mockBridge(FIXTURE);
  const bridge: Bridge = {
    ...base,
    native: true,
    clientStateMaintenanceStatus: async () => snapshot,
    onMaintenanceStatus: async () => () => {},
    ...overrides,
  };
  const rendered = ui.render(createElement(App, { bridge }));
  await ui.waitFor(() => {
    assert.ok(document.querySelector('.stopcard'));
  });
  assert.equal(document.querySelectorAll('.takeover').length, 1);
  assert.ok(document.querySelector('.window > .takeover'));
  assert.equal(
    document.querySelector('.takeover')?.hasAttribute('tabindex'),
    false,
  );
  return { rendered, bridge };
}

async function shellOverlay(
  snapshot: MaintenanceSnapshot,
  overrides: Partial<Bridge> = {},
) {
  const { App, FIXTURE, mockBridge } = await modules();
  const base = mockBridge(FIXTURE);
  const listeners = new Set<(value: MaintenanceSnapshot) => void>();
  let current: MaintenanceSnapshot = {
    state: 'idle',
    generation: 0,
    revision: 0,
  };
  const bridge: Bridge = {
    ...base,
    native: true,
    clientStateMaintenanceStatus: async () => current,
    onMaintenanceStatus: async (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    ...overrides,
  };
  const rendered = ui.render(createElement(App, { bridge }));
  await ui.waitFor(() => {
    assert.ok(document.querySelector('.side.rail .who .t'));
  });
  current = snapshot;
  await ui.act(async () => {
    for (const listener of listeners) listener(snapshot);
    await Promise.resolve();
  });
  await ui.waitFor(() => {
    assert.ok(document.querySelector('.stopveil'));
  });
  assert.equal(document.querySelectorAll('.takeover').length, 1);
  assert.ok(document.querySelector('.window > .takeover'));
  assert.equal(
    document.querySelector('.takeover')?.getAttribute('role'),
    'alertdialog',
  );
  assert.equal(
    document.querySelector('.takeover')?.getAttribute('tabindex'),
    '-1',
  );
  return { rendered, bridge };
}

async function primitiveBlock(
  kind: 'popover' | 'context',
  initialBlocking: boolean,
): Promise<void> {
  const { ContextMenu, Dialog, Menu, OverlayProvider, Popover } =
    (await vite.ssrLoadModule(
      '/kit/overlay-primitives.tsx',
    )) as typeof import('../kit/overlay-primitives');
  const portalRoot = document.getElementById('overlays');
  const background = document.getElementById('root');
  assert.ok(portalRoot);
  assert.ok(background);
  const backgroundRef = { current: background };
  let closes = 0;

  function Host() {
    const [open, setOpen] = useState(true);
    const anchorRef = useRef<HTMLButtonElement>(null);
    const close = (): void => {
      closes++;
      setOpen(false);
    };
    const portal = !open
      ? null
      : kind === 'popover'
        ? createElement(Popover, {
            anchorRef,
            className: 'primitive-portal',
            onClose: close,
            children: createElement(Menu, {
              anchorRef,
              onClose: close,
              children: createElement('button', {}, 'Choice'),
            }),
          })
        : createElement(ContextMenu, {
            point: { x: 20, y: 20 },
            className: 'primitive-portal',
            onClose: close,
            children: 'Choice',
          });
    return createElement(
      'div',
      { className: 'primitive-host' },
      createElement(Dialog, {
        'aria-label': 'Persistent dialog',
        children: 'Persistent dialog',
      }),
      createElement('button', { ref: anchorRef }, 'Anchor'),
      portal,
    );
  }

  const draw = (blocking: boolean) =>
    createElement(OverlayProvider, {
      backgroundRef,
      portalRoot,
      blocking,
      children: createElement(Host),
    });
  const rendered = ui.render(draw(initialBlocking));
  if (!initialBlocking)
    await ui.waitFor(() =>
      assert.ok(document.querySelector('.primitive-portal')),
    );
  const dialog = await rendered.findByRole('dialog', {
    name: 'Persistent dialog',
  });
  assert.equal(background.inert, true);
  if (!initialBlocking) rendered.rerender(draw(true));
  await ui.waitFor(() => {
    assert.equal(closes, 1);
    assert.equal(document.querySelector('.primitive-portal'), null);
  });
  assert.ok(
    document.querySelector('[aria-label="Persistent dialog"]') === dialog,
  );
  assert.equal(background.inert, true);
  const host = document.querySelector('.primitive-host');
  assert.ok(host);
  await ui.act(async () => {
    ui.fireEvent.pointerDown(host);
    ui.fireEvent.keyDown(host, { key: 'Escape' });
    await Promise.resolve();
  });
  assert.equal(closes, 1);
  rendered.rerender(draw(true));
  assert.equal(closes, 1);
  rendered.rerender(draw(false));
  assert.equal(closes, 1);
  assert.equal(document.querySelector('.primitive-portal'), null);
  assert.ok(
    document.querySelector('[aria-label="Persistent dialog"]') === dialog,
  );
  assert.equal(background.inert, true);
  rendered.unmount();
}

test('popover and context menu close once when overlays become blocked', async () => {
  await primitiveBlock('popover', false);
  await primitiveBlock('context', false);
});

test('popover and context menu suppress blocked initial mounts', async () => {
  await primitiveBlock('popover', true);
  await primitiveBlock('context', true);
});

test('initial startup uses the shared takeover frame', async () => {
  const { App, FIXTURE, mockBridge } = await modules();
  const lock = deferred<Awaited<ReturnType<Bridge['appLockState']>>>();
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    appLockState: () => lock.promise,
  };
  const rendered = ui.render(createElement(App, { bridge }));
  const status = await rendered.findByRole('status');
  const takeover = status.closest('.takeover');
  assert.ok(takeover);
  assert.equal(document.querySelectorAll('.takeover').length, 1);
  assert.equal(takeover.parentElement, document.querySelector('.window'));
  rendered.unmount();
});

test('locked startup outranks maintenance and keeps unlock errors in its card', async () => {
  const { App, FIXTURE, mockBridge } = await modules();
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    clientStateMaintenanceStatus: async () => ({
      state: 'complete',
      generation: 2,
      revision: 4,
      kind: 'relocate',
      operation: { status: 'completed' },
      disposition: {
        status: 'restart-selected-root',
        root: '/moved/foks-rs',
      },
    }),
    appLockState: async () => ({
      locked: true,
      available: true,
      mechanism: 'password',
    }),
    unlockApp: async () => {
      throw new Error('Authentication was refused.');
    },
  };
  const rendered = ui.render(createElement(App, { bridge }));
  const dialog = await rendered.findByRole('dialog', {
    name: 'FOKS is locked',
  });
  assert.ok(dialog.classList.contains('takeover'));
  assert.ok(dialog.classList.contains('lock-back'));
  assert.equal(dialog.parentElement, document.querySelector('.window'));
  assert.equal(document.querySelector('.stopcard'), null);
  assert.equal(Boolean(document.querySelector('.side.rail .status')), false);
  await ui.act(async () => {
    rendered.getByRole('button', { name: 'Unlock' }).click();
    await Promise.resolve();
  });
  const error = await rendered.findByRole('alert');
  assert.match(error.textContent ?? '', /Authentication was refused/);
  assert.ok(dialog.contains(error));
  rendered.unmount();
});

test('boot error retry restarts startup', async () => {
  const { App, FIXTURE, mockBridge } = await modules();
  let statusCalls = 0;
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    agentStatus: async () => {
      statusCalls++;
      if (statusCalls === 1) throw new Error('The agent status failed.');
      return { state: 'ready' };
    },
  };
  const rendered = ui.render(createElement(App, { bridge }));
  const dialog = await rendered.findByRole('alertdialog', {
    name: 'Couldn’t load FOKS',
  });
  assert.ok(dialog.classList.contains('takeover'));
  assert.ok(dialog.classList.contains('lock-back'));
  assert.match(dialog.textContent ?? '', /The agent status failed/);
  await ui.act(async () => {
    rendered.getByRole('button', { name: 'Retry' }).click();
    await Promise.resolve();
  });
  await ui.waitFor(() =>
    assert.equal(document.querySelector('.takeover'), null),
  );
  assert.ok(document.querySelector('.side.rail'));
  assert.equal(Boolean(document.querySelector('.side.rail .status')), false);
  assert.ok(statusCalls >= 2);
  rendered.unmount();
});

test('the agent-lost dialog shows the reason it was given', async () => {
  const { rendered } = await agentLostOverlay({
    message: 'The agent socket closed while reading the catalog.',
  });
  const dialog = document.querySelector('.stopwrap');
  assert.ok(dialog);
  assert.ok(dialog.classList.contains('takeover'));
  assert.equal(document.querySelectorAll('.takeover').length, 1);
  assert.match(
    dialog.textContent ?? '',
    /The agent socket closed while reading the catalog\./,
  );
  rendered.unmount();
});

test('a failed agent-lost retry is reported inside the dialog', async () => {
  const { rendered } = await agentLostOverlay({
    message: 'The agent socket closed while reading the catalog.',
    retryAgentConnection: async () => {
      throw new Error('The background service did not restart.');
    },
  });
  const retry = await rendered.findByRole('button', { name: 'Retry' });
  await ui.act(async () => {
    retry.click();
    await Promise.resolve();
  });
  await ui.waitFor(() => {
    assert.match(
      document.querySelector('.stopwrap')?.textContent ?? '',
      /The background service did not restart\./,
    );
  });
  assert.ok(document.querySelector('.stopwrap [role="alert"]'));
  rendered.unmount();
});

test('the agent-lost dialog quits through the bridge', async () => {
  let quits = 0;
  const { rendered } = await agentLostOverlay({
    quit: async () => {
      quits++;
    },
  });
  const quit = await rendered.findByRole('button', { name: 'Quit FOKS' });
  await ui.act(async () => {
    quit.click();
    await Promise.resolve();
  });
  assert.equal(quits, 1);
  rendered.unmount();
});

const restartRequired: MaintenanceSnapshot = {
  state: 'complete',
  generation: 2,
  revision: 4,
  kind: 'relocate',
  operation: { status: 'completed' },
  disposition: { status: 'restart-selected-root', root: '/moved/foks-rs' },
};

test('restart-required restarts the app before the shell mounts', async () => {
  let restarts = 0;
  const { rendered } = await startupOverlay(restartRequired, {
    restartApp: async () => {
      restarts++;
    },
  });
  assert.match(
    document.body.textContent ?? '',
    /Please restart FOKS to open \/moved\/foks-rs\./,
  );
  const restart = await rendered.findByRole('button', { name: 'Restart FOKS' });
  await ui.act(async () => {
    restart.click();
    await Promise.resolve();
  });
  assert.equal(restarts, 1);
  rendered.unmount();
});

test('restart-required restarts the app from the mounted shell', async () => {
  let restarts = 0;
  const { rendered } = await shellOverlay(restartRequired, {
    restartApp: async () => {
      restarts++;
    },
  });
  assert.match(
    document.querySelector('.stopveil')?.textContent ?? '',
    /Please restart FOKS to open \/moved\/foks-rs\./,
  );
  const restart = await rendered.findByRole('button', { name: 'Restart FOKS' });
  await ui.act(async () => {
    restart.click();
    await Promise.resolve();
  });
  assert.equal(restarts, 1);
  rendered.unmount();
});

test('recovery-required displays copyable commands and a quit button', async () => {
  const copied: string[] = [];
  let quits = 0;
  const { rendered } = await startupOverlay(
    {
      state: 'complete',
      generation: 3,
      revision: 5,
      kind: 'relocate',
      operation: { status: 'cancelled' },
      disposition: { status: 'recovery-required', root: '/old/foks-rs' },
    },
    {
      copyText: async (text: string) => {
        copied.push(text);
        return { ok: true };
      },
      quitApp: async () => {
        quits++;
      },
    },
  );
  const commands = [...document.querySelectorAll('.copybox code')].map(
    (node) => node.textContent,
  );
  assert.deepEqual(commands, [
    'foks-rs --state-dir /old/foks-rs state status',
    'foks-rs --state-dir /old/foks-rs state recover',
  ]);
  const copy = rendered.getAllByRole('button', { name: 'Copy' });
  assert.equal(copy.length, 2);
  await ui.act(async () => {
    copy[1].click();
    await Promise.resolve();
  });
  assert.deepEqual(copied, ['foks-rs --state-dir /old/foks-rs state recover']);
  const quit = await rendered.findByRole('button', { name: 'Quit FOKS' });
  await ui.act(async () => {
    quit.click();
    await Promise.resolve();
  });
  assert.equal(quits, 1);
  rendered.unmount();
});

test('restoration-failed keeps its retry and adds a quit action', async () => {
  let retries = 0;
  let quits = 0;
  const { rendered } = await startupOverlay(
    {
      state: 'complete',
      generation: 4,
      revision: 6,
      kind: 'verify',
      operation: { status: 'completed' },
      disposition: {
        status: 'restoration-failed',
        root: '/isolated/client',
        error: {
          code: 'agent-stop-pending',
          message: 'The prior agent is still exiting.',
          retryable: true,
          ambiguous: false,
          fatal: false,
        },
      },
    },
    {
      retryAgentConnection: async () => {
        retries++;
        return { state: 'ready' };
      },
      quitApp: async () => {
        quits++;
      },
    },
  );
  assert.match(
    document.body.textContent ?? '',
    /The prior agent is still exiting\./,
  );
  const quit = await rendered.findByRole('button', { name: 'Quit FOKS' });
  await ui.act(async () => {
    quit.click();
    await Promise.resolve();
  });
  assert.equal(quits, 1);
  const retry = await rendered.findByRole('button', {
    name: 'Restart service',
  });
  await ui.act(async () => {
    retry.click();
    await Promise.resolve();
  });
  await ui.waitFor(() => assert.equal(retries, 1));
  rendered.unmount();
});

test('a conceal does not reopen the sheet a scene opened with the page', async () => {
  const { App, FIXTURE, mockBridge } = await modules();
  const listeners = new Set<(value: MaintenanceSnapshot) => void>();
  let current: MaintenanceSnapshot = {
    state: 'idle',
    generation: 0,
    revision: 0,
  };
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    clientStateMaintenanceStatus: async () => current,
    onMaintenanceStatus: async (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
  // `?state=settings-phrase` opens Devices with the one-time paper-key sheet.
  window.history.replaceState(null, '', '/?state=settings-phrase');
  const rendered = ui.render(createElement(App, { bridge }));
  await ui.waitFor(() => {
    assert.match(document.body.textContent ?? '', /Save paper key/);
  });

  // The conceal remounts the tabs to take the phrase off the screen. The scene
  // was entered once: re-reading it here would put the phrase straight back.
  current = {
    state: 'complete',
    generation: 6,
    revision: 8,
    kind: 'verify',
    operation: { status: 'completed' },
    disposition: { status: 'continue-current-root' },
  };
  await ui.act(async () => {
    for (const listener of listeners) listener(current);
    await Promise.resolve();
  });
  await ui.waitFor(() => {
    assert.doesNotMatch(document.body.textContent ?? '', /Save paper key/);
  });
  // The page itself is back, so the phrase is gone rather than the whole tab.
  assert.match(document.body.textContent ?? '', /paper-backup/);
  rendered.unmount();
});

test('in-shell stop notice renders outside the inert main container', async () => {
  const { rendered } = await shellOverlay(restartRequired);
  const dialog = document.querySelector('.stopveil');
  assert.ok(dialog);
  // The dialog isolates the shell grid; a notice inside that grid would be
  // inert along with it and its Restart and Quit buttons unreachable.
  assert.equal(
    document.querySelector('.app')?.getAttribute('aria-hidden'),
    'true',
  );
  assert.equal(dialog.closest('[aria-hidden="true"]'), null);
  assert.ok(document.querySelector('.window > .stopveil'));
  // The rail and topbar beside the veil are dimmed and disabled.
  assert.ok(document.querySelector('.rail-body.is-blocked'));
  for (const tab of document.querySelectorAll('.rail-tabs .nav'))
    assert.ok(tab.hasAttribute('disabled'));
  rendered.unmount();
});

test('lost-connection dialog renders beside a dimmed navigation rail', async () => {
  const { rendered } = await agentLostOverlay({});
  const dialog = document.querySelector('.stopwrap');
  assert.ok(dialog);
  assert.equal(dialog.closest('[aria-hidden="true"]'), null);
  assert.ok(document.querySelector('.window > .stopwrap'));
  assert.ok(document.querySelector('.side.rail .rail-tabs'));
  assert.ok(document.querySelector('.rail-body.is-blocked'));
  assert.ok(
    document.querySelector('.topbar .topsearch')?.hasAttribute('disabled'),
  );
  rendered.unmount();
});

test('agent loss preserves a new password draft through retry', async () => {
  const { App, FIXTURE, mockBridge } = await modules();
  const loss = deferred<string | null>();
  const retry = deferred<Awaited<ReturnType<Bridge['retryAgentConnection']>>>();
  let writes = 0;
  let lossConsumed = false;
  window.history.replaceState(null, '', '/?state=new');
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    takeAgentConnectionLoss: () => {
      if (lossConsumed) return Promise.resolve(null);
      return loss.promise.then((message) => {
        lossConsumed = true;
        return message;
      });
    },
    retryAgentConnection: () => retry.promise,
    createTextItem: async () => {
      writes++;
      return { applied: true };
    },
  };
  const rendered = ui.render(createElement(App, { bridge }));
  const site = (await rendered.findByLabelText('Site')) as HTMLInputElement;
  const username = rendered.getByLabelText('User name') as HTMLInputElement;
  const password = rendered.getByLabelText('Password') as HTMLInputElement;
  const website = rendered.getByLabelText('Website') as HTMLInputElement;
  ui.fireEvent.change(site, { target: { value: 'draft.example' } });
  ui.fireEvent.change(username, { target: { value: 'draft-user' } });
  ui.fireEvent.change(password, { target: { value: 'draft-password' } });
  ui.fireEvent.change(website, { target: { value: 'https://draft.example' } });
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Advanced' }));
  const path = rendered.getByLabelText('Path') as HTMLInputElement;
  ui.fireEvent.change(path, { target: { value: '/logins/preserved' } });
  site.focus();
  const sheet = document.querySelector('[aria-label="New password"]');
  const app = document.querySelector<HTMLElement>('.app');
  assert.ok(sheet);
  assert.ok(app);

  await ui.act(async () => {
    loss.resolve('The agent socket closed while reading the catalog.');
    await Promise.resolve();
  });
  const blocker = await rendered.findByRole('alertdialog', {
    name: 'Connection to background service lost',
  });
  assert.equal(document.querySelector('[aria-label="New password"]'), sheet);
  assert.equal(rendered.getByLabelText('Site'), site);
  assert.deepEqual(
    [site.value, username.value, password.value, website.value, path.value],
    [
      'draft.example',
      'draft-user',
      'draft-password',
      'https://draft.example',
      '/logins/preserved',
    ],
  );
  assert.equal(sheet.getAttribute('aria-hidden'), 'true');
  assert.ok((sheet as HTMLElement).inert);
  assert.equal(document.querySelector('.app'), app);
  assert.equal(app.inert, true);
  assert.equal(app.getAttribute('aria-hidden'), 'true');
  const retryButton = ui.within(blocker).getByRole('button', { name: 'Retry' });
  assert.equal(retryButton.closest('[inert]'), null);

  await ui.act(async () => {
    retryButton.click();
    await Promise.resolve();
  });
  assert.ok(document.querySelector('.stopwrap'));
  assert.equal(retryButton.hasAttribute('disabled'), true);
  await ui.act(async () => {
    retry.resolve({ state: 'ready' });
    await retry.promise;
    await Promise.resolve();
  });
  assert.equal(document.querySelector('.stopwrap'), null);
  assert.equal(document.querySelector('[aria-label="New password"]'), sheet);
  assert.equal(rendered.getByLabelText('Site'), site);
  assert.deepEqual(
    [site.value, username.value, password.value, website.value, path.value],
    [
      'draft.example',
      'draft-user',
      'draft-password',
      'https://draft.example',
      '/logins/preserved',
    ],
  );
  assert.equal((sheet as HTMLElement).inert, false);
  assert.equal(sheet.hasAttribute('aria-hidden'), false);
  assert.equal(document.querySelector('.app'), app);
  assert.equal(app.inert, true);
  assert.equal(app.getAttribute('aria-hidden'), 'true');
  site.focus();
  assert.equal(document.activeElement, site);
  assert.equal(writes, 0);

  ui.fireEvent.mouseDown(sheet);
  const discard = await rendered.findByRole('button', { name: 'Discard' });
  await ui.act(async () => {
    discard.click();
    await Promise.resolve();
  });
  await ui.waitFor(() =>
    assert.equal(document.querySelector('[aria-label="New password"]'), null),
  );
  assert.equal(document.querySelector('.app'), app);
  assert.equal(app.inert, false);
  assert.equal(app.hasAttribute('aria-hidden'), false);
  rendered.unmount();
});

test('a failed pending retry can be retried successfully', async () => {
  const first = deferred<Awaited<ReturnType<Bridge['retryAgentConnection']>>>();
  let attempts = 0;
  const { rendered } = await agentLostOverlay({
    retryAgentConnection: () => {
      attempts++;
      return attempts === 1
        ? first.promise
        : Promise.resolve({ state: 'ready' });
    },
  });
  const retry = await rendered.findByRole('button', { name: 'Retry' });
  await ui.act(async () => {
    retry.click();
    await Promise.resolve();
  });
  assert.equal(retry.hasAttribute('disabled'), true);
  first.reject(new Error('The background service did not restart.'));
  await ui.waitFor(() => assert.equal(retry.hasAttribute('disabled'), false));
  assert.ok(document.querySelector('.stopwrap [role="alert"]'));
  await ui.act(async () => {
    retry.click();
    await Promise.resolve();
  });
  await ui.waitFor(() =>
    assert.equal(document.querySelector('.stopwrap'), null),
  );
  assert.equal(attempts, 2);
  rendered.unmount();
});

test('catalog failure after reconnect reports a toast with the agent ready', async () => {
  const { rendered, retryCatalog, catalogEntered } =
    await agentLostWithDeferredCatalog();
  await ui.act(async () => {
    rendered.getByRole('button', { name: 'Retry' }).click();
    await Promise.resolve();
  });
  await catalogEntered.promise;
  await ui.waitFor(() =>
    assert.equal(document.querySelector('.stopwrap'), null),
  );
  await ui.act(async () => {
    retryCatalog.reject(new Error('The catalog could not be refreshed.'));
    await Promise.resolve();
  });
  await rendered.findByText('The catalog could not be refreshed.');
  assert.ok(document.querySelector('.side.rail'));
  assert.equal(Boolean(document.querySelector('.side.rail .status')), false);
  assert.equal(document.querySelector('.stopwrap'), null);
  rendered.unmount();
});

test('agent loss from the reconnect catalog restores the takeover', async () => {
  const { rendered, retryCatalog, catalogEntered } =
    await agentLostWithDeferredCatalog();
  const first = document.querySelector('.stopwrap');
  assert.ok(first);
  await ui.act(async () => {
    rendered.getByRole('button', { name: 'Retry' }).click();
    await Promise.resolve();
  });
  await catalogEntered.promise;
  await ui.waitFor(() =>
    assert.equal(document.querySelector('.stopwrap'), null),
  );
  await ui.act(async () => {
    retryCatalog.reject({
      code: 'agent-lost',
      message: 'The agent disconnected during catalog refresh.',
      retryable: true,
      ambiguous: false,
      fatal: true,
    });
    await Promise.resolve();
  });
  const second = await rendered.findByRole('alertdialog', {
    name: 'Connection to background service lost',
  });
  assert.notEqual(second, first);
  assert.match(second.textContent ?? '', /disconnected during catalog refresh/);
  assert.equal(Boolean(document.querySelector('.side.rail .status')), false);
  rendered.unmount();
});

test('maintenance cannot be overwritten by duplicate agent loss', async () => {
  const { App, FIXTURE, mockBridge } = await modules();
  const listeners = new Set<(value: MaintenanceSnapshot) => void>();
  let lossCalls = 0;
  window.history.replaceState(null, '', '/?state=new');
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    clientStateMaintenanceStatus: async () => ({
      state: 'idle',
      generation: 0,
      revision: 0,
    }),
    onMaintenanceStatus: async (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    takeAgentConnectionLoss: async () => {
      lossCalls++;
      return lossCalls <= 2 ? `socket event ${lossCalls}` : null;
    },
  };
  const rendered = ui.render(createElement(App, { bridge }));
  const sheet = await rendered.findByLabelText('Site');
  await rendered.findByRole('alertdialog', {
    name: 'Connection to background service lost',
  });
  const active: MaintenanceSnapshot = {
    state: 'active',
    generation: 1,
    revision: 1,
    kind: 'export',
    phase: 'running',
  };
  await ui.act(async () => {
    for (const listener of listeners) listener(active);
    await Promise.resolve();
  });
  await ui.waitFor(() => assert.ok(document.querySelector('.stopveil')));
  await ui.waitFor(() => assert.ok(lossCalls >= 2), { timeout: 1500 });
  assert.equal(document.querySelectorAll('.stopveil, .stopwrap').length, 1);
  assert.ok(document.querySelector('.stopveil'));
  assert.equal(document.querySelector('.stopveil')?.closest('[inert]'), null);

  const complete: MaintenanceSnapshot = {
    state: 'complete',
    generation: 1,
    revision: 2,
    kind: 'export',
    operation: { status: 'completed' },
    disposition: { status: 'continue-current-root' },
  };
  await ui.act(async () => {
    for (const listener of listeners) listener(complete);
    await Promise.resolve();
  });
  await ui.waitFor(() =>
    assert.equal(document.querySelector('.stopveil, .stopwrap'), null),
  );
  assert.equal(rendered.getByLabelText('Site'), sheet);
  rendered.unmount();
});

test('takeover transitions restore focus without releasing retained isolation', async () => {
  const { App, FIXTURE, mockBridge } = await modules();
  const listeners = new Set<(value: MaintenanceSnapshot) => void>();
  let current: MaintenanceSnapshot = {
    state: 'idle',
    generation: 0,
    revision: 0,
  };
  const connectionLoss = deferred<string | null>();
  let lossConsumed = false;
  let retries = 0;
  let restarts = 0;
  let quits = 0;
  window.history.replaceState(null, '', '/?state=new');
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    clientStateMaintenanceStatus: async () => current,
    onMaintenanceStatus: async (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    takeAgentConnectionLoss: () => {
      if (lossConsumed) return Promise.resolve(null);
      return connectionLoss.promise.then((message) => {
        lossConsumed = true;
        return message;
      });
    },
    retryAgentConnection: async () => {
      retries++;
      return { state: 'ready' };
    },
    restartApp: async () => {
      restarts++;
    },
    quitApp: async () => {
      quits++;
    },
  };
  const rendered = ui.render(createElement(App, { bridge }));
  const input = await rendered.findByLabelText('Site');
  const sheet = input.closest<HTMLElement>('[aria-label="New password"]');
  assert.ok(sheet);
  await ui.act(async () => {
    connectionLoss.resolve('The agent socket closed.');
    await Promise.resolve();
  });
  const retry = await rendered.findByRole('button', { name: 'Retry' });
  const app = document.querySelector<HTMLElement>('.app');
  assert.ok(app);
  assert.equal(document.activeElement, retry);
  assert.equal(app.inert, true);
  assert.equal(sheet.inert, true);

  current = {
    state: 'active',
    generation: 7,
    revision: 1,
    kind: 'export',
    phase: 'running',
  };
  await ui.act(async () => {
    for (const listener of listeners) listener(current);
    await Promise.resolve();
  });
  const maintenance = await rendered.findByRole('alertdialog', {
    name: 'export · running',
  });
  assert.ok(document.activeElement === maintenance);
  assert.equal(document.querySelectorAll('.takeover').length, 1);
  assert.equal(
    document.querySelectorAll('.takeover:not([aria-hidden="true"])').length,
    1,
  );
  assert.equal(app.inert, true);
  assert.equal(sheet.inert, true);
  assert.equal(maintenance.querySelectorAll('button').length, 0);

  current = {
    state: 'complete',
    generation: 7,
    revision: 2,
    kind: 'export',
    operation: { status: 'completed' },
    disposition: { status: 'restart-selected-root', root: '/moved/foks-rs' },
  };
  await ui.act(async () => {
    for (const listener of listeners) listener(current);
    await Promise.resolve();
  });
  const restart = await rendered.findByRole('button', { name: 'Restart FOKS' });
  assert.ok(document.activeElement === restart);
  assert.equal(document.querySelectorAll('.takeover').length, 1);
  assert.equal(
    document.querySelectorAll('.takeover:not([aria-hidden="true"])').length,
    1,
  );
  assert.equal(app.inert, true);
  assert.equal(sheet.inert, true);
  assert.deepEqual(
    { retries, restarts, quits },
    { retries: 0, restarts: 0, quits: 0 },
  );
  rendered.unmount();
});

test('command-only agent loss preserves the active write workflow', async () => {
  const { App, FIXTURE, mockBridge } = await modules();
  window.history.replaceState(null, '', '/?state=new');
  const lost = {
    code: 'agent-lost',
    message: 'The create command lost its agent connection.',
    retryable: true,
    ambiguous: false,
    fatal: true,
  };
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    takeAgentConnectionLoss: async () => null,
    createTextItem: async () => {
      throw lost;
    },
  };
  const rendered = ui.render(createElement(App, { bridge }));
  const site = (await rendered.findByLabelText('Site')) as HTMLInputElement;
  ui.fireEvent.change(site, { target: { value: 'command.example' } });
  const sheet = document.querySelector('[aria-label="New password"]');
  assert.ok(sheet);
  await ui.act(async () => {
    rendered.getByRole('button', { name: 'Create item' }).click();
    await Promise.resolve();
  });
  await rendered.findByRole('alertdialog', {
    name: 'Connection to background service lost',
  });
  assert.match(
    document.querySelector('.stopwrap')?.textContent ?? '',
    /create command/,
  );
  assert.equal(Boolean(document.querySelector('.side.rail .status')), false);
  assert.equal(document.querySelector('[aria-label="New password"]'), sheet);
  assert.equal(rendered.getByLabelText('Site'), site);
  assert.equal(site.value, 'command.example');
  rendered.unmount();
});

test('agent loss closes the rail account menu before retry interaction', async () => {
  const { App, FIXTURE, mockBridge } = await modules();
  const loss = deferred<string | null>();
  let consumed = false;
  let retries = 0;
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    takeAgentConnectionLoss: () => {
      if (consumed) return Promise.resolve(null);
      return loss.promise.then((message) => {
        consumed = true;
        return message;
      });
    },
    retryAgentConnection: async () => {
      retries++;
      return { state: 'ready' };
    },
  };
  const rendered = ui.render(createElement(App, { bridge }));
  const account = await ui.waitFor(() => {
    const button = document.querySelector<HTMLButtonElement>('button.who');
    assert.ok(button);
    assert.equal(button.disabled, false);
    return button;
  });
  ui.fireEvent.click(account);
  await rendered.findByRole('menu');
  await ui.act(async () => {
    loss.resolve('The agent socket closed.');
    await Promise.resolve();
  });
  await ui.waitFor(() =>
    assert.equal(document.querySelector('[role="menu"]'), null),
  );
  const retry = await rendered.findByRole('button', { name: 'Retry' });
  assert.equal(document.activeElement, retry);
  await ui.act(async () => {
    ui.fireEvent.pointerDown(retry);
    ui.fireEvent.mouseDown(retry);
    ui.fireEvent.click(retry);
    await Promise.resolve();
  });
  await ui.waitFor(() =>
    assert.equal(document.querySelector('.takeover'), null),
  );
  assert.equal(retries, 1);
  assert.equal(document.querySelector('[role="menu"]'), null);
  rendered.unmount();
});

test('agent loss closes the new-item store popover without replacing its draft', async () => {
  const { App, FIXTURE, mockBridge } = await modules();
  const loss = deferred<string | null>();
  let consumed = false;
  let retries = 0;
  window.history.replaceState(null, '', '/?state=new');
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    takeAgentConnectionLoss: () => {
      if (consumed) return Promise.resolve(null);
      return loss.promise.then((message) => {
        consumed = true;
        return message;
      });
    },
    retryAgentConnection: async () => {
      retries++;
      return { state: 'ready' };
    },
  };
  const rendered = ui.render(createElement(App, { bridge }));
  const input = (await rendered.findByLabelText('Site')) as HTMLInputElement;
  ui.fireEvent.change(input, { target: { value: 'retained.example' } });
  const sheet = input.closest('[aria-label="New password"]');
  const trigger = rendered.getByRole('button', { name: 'Save in vault' });
  const selected = trigger.textContent;
  assert.ok(sheet);
  ui.fireEvent.click(trigger);
  await rendered.findByRole('listbox', { name: 'Save in vault' });
  await ui.act(async () => {
    loss.resolve('The agent socket closed.');
    await Promise.resolve();
  });
  await ui.waitFor(() =>
    assert.equal(document.querySelector('[role="listbox"]'), null),
  );
  const retry = await rendered.findByRole('button', { name: 'Retry' });
  assert.equal(document.querySelector('[aria-label="New password"]'), sheet);
  assert.equal(rendered.getByLabelText('Site'), input);
  assert.equal(input.value, 'retained.example');
  assert.equal(retry.closest('[inert]'), null);
  await ui.act(async () => {
    retry.click();
    await Promise.resolve();
  });
  await ui.waitFor(() =>
    assert.equal(document.querySelector('.takeover'), null),
  );
  assert.equal(retries, 1);
  assert.equal(document.querySelector('[role="listbox"]'), null);
  assert.equal(trigger.textContent, selected);
  assert.equal(input.value, 'retained.example');
  rendered.unmount();
});

test('agent loss closes the search palette until a later shortcut reopens it', async () => {
  const { App, FIXTURE, mockBridge } = await modules();
  const loss = deferred<string | null>();
  let consumed = false;
  let retries = 0;
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    takeAgentConnectionLoss: () => {
      if (consumed) return Promise.resolve(null);
      return loss.promise.then((message) => {
        consumed = true;
        return message;
      });
    },
    retryAgentConnection: async () => {
      retries++;
      return { state: 'ready' };
    },
  };
  const rendered = ui.render(createElement(App, { bridge }));
  const search = await ui.waitFor(() => {
    const button = document.querySelector<HTMLButtonElement>('.topsearch');
    assert.ok(button);
    assert.equal(button.disabled, false);
    return button;
  });
  ui.fireEvent.click(search);
  await ui.waitFor(() => assert.ok(document.querySelector('.pal-back')));
  await ui.act(async () => {
    loss.resolve('The agent socket closed.');
    await Promise.resolve();
  });
  await ui.waitFor(() => {
    assert.equal(document.querySelector('.pal-back'), null);
    assert.equal(document.querySelector('.pal-q'), null);
  });
  const retry = await rendered.findByRole('button', { name: 'Retry' });
  assert.equal(document.activeElement, retry);
  ui.fireEvent.keyDown(document, { key: 'k', metaKey: true });
  assert.equal(document.querySelector('.pal-back'), null);
  await ui.act(async () => {
    retry.click();
    await Promise.resolve();
  });
  await ui.waitFor(() =>
    assert.equal(document.querySelector('.takeover'), null),
  );
  assert.equal(retries, 1);
  assert.equal(document.querySelector('.pal-back'), null);
  ui.fireEvent.keyDown(document, { key: 'k', metaKey: true });
  await ui.waitFor(() => assert.ok(document.querySelector('.pal-back')));
  rendered.unmount();
});

test('first-run loss blocks the setup rail without adding a topbar', async () => {
  const { App, FIXTURE, mockBridge } = await modules();
  let connectionLoss: string | null = 'The first-run agent disconnected.';
  window.history.replaceState(null, '', '/?state=first-run&step=who');
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    takeAgentConnectionLoss: async () => {
      const pending = connectionLoss;
      connectionLoss = null;
      return pending;
    },
  };
  const rendered = ui.render(createElement(App, { snapshot: FIXTURE, bridge }));
  await rendered.findByRole('alertdialog', {
    name: 'Connection to background service lost',
  });
  assert.ok(document.querySelector('.first-run-main'));
  assert.equal(document.querySelector('.topbar'), null);
  assert.ok(document.querySelector('.setup-steps.is-blocked'));
  assert.equal(Boolean(document.querySelector('.setup-side .status')), false);
  assert.equal(Boolean(document.querySelector('.setup-side .foot')), false);
  assert.ok(document.querySelector('.window > .takeover.stopwrap'));
  for (const button of document.querySelectorAll('.setup-side button'))
    assert.ok((button as HTMLButtonElement).disabled);
  rendered.unmount();
});

test('first-run maintenance blocks the setup rail without a connection footer', async () => {
  const { App, FIXTURE, mockBridge } = await modules();
  window.history.replaceState(null, '', '/?state=first-run&step=who');
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    clientStateMaintenanceStatus: async () => ({
      state: 'active',
      generation: 3,
      revision: 1,
      kind: 'verify',
      phase: 'running',
    }),
    onMaintenanceStatus: async () => () => {},
  };
  const rendered = ui.render(createElement(App, { snapshot: FIXTURE, bridge }));
  await ui.waitFor(() => assert.ok(document.querySelector('.stopveil')));
  assert.ok(document.querySelector('.first-run-main'));
  assert.equal(document.querySelector('.topbar'), null);
  assert.ok(document.querySelector('.setup-steps.is-blocked'));
  assert.equal(Boolean(document.querySelector('.setup-side .status')), false);
  assert.equal(Boolean(document.querySelector('.setup-side .foot')), false);
  assert.ok(document.querySelector('.window > .takeover.stopveil'));
  for (const button of document.querySelectorAll('.setup-side button'))
    assert.ok((button as HTMLButtonElement).disabled);
  rendered.unmount();
});

test('active maintenance overlay renders no action buttons', async () => {
  const { rendered } = await shellOverlay({
    state: 'active',
    generation: 5,
    revision: 7,
    kind: 'export',
    phase: 'running',
  });
  const dialog = document.querySelector('.stopveil');
  assert.ok(dialog);
  assert.match(dialog.textContent ?? '', /export · running/i);
  assert.equal(dialog.querySelectorAll('button').length, 0);
  rendered.unmount();
});

test('keychain-blocked automatic recovery offers explicit restoration and survives a denied retry', async () => {
  const { App, FIXTURE, mockBridge } = await modules();
  let loss = true;
  let automatic = 0;
  let manual = 0;
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    takeAgentConnectionLoss: async () => {
      if (!loss) return null;
      loss = false;
      return 'Agent endpoint closed';
    },
    autoRecoverAgent: async () => {
      automatic++;
      throw {
        code: 'agent-credentials-required',
        message: 'Keychain access is needed to restore your connection.',
        retryable: false,
        fatal: false,
        ambiguous: false,
      };
    },
    retryAgentConnection: async () => {
      manual++;
      if (manual === 1) throw new Error('Keychain authorization was denied.');
      return { state: 'ready' };
    },
  };
  const rendered = ui.render(createElement(App, { snapshot: FIXTURE, bridge }));
  const restore = await rendered.findByRole('button', {
    name: 'Restore connection',
  });
  assert.equal(manual, 0);
  assert.equal(automatic, 1);
  await ui.act(async () => {
    restore.click();
  });
  await ui.waitFor(() =>
    assert.match(
      document.querySelector('.stopwrap [role="alert"]')?.textContent ?? '',
      /Keychain authorization was denied/,
    ),
  );
  await ui.act(async () => {
    rendered.getByRole('button', { name: 'Restore connection' }).click();
  });
  await ui.waitFor(() =>
    assert.equal(document.querySelector('.stopwrap'), null),
  );
  assert.equal(manual, 2);
  assert.equal(automatic, 1);
  rendered.unmount();
});
