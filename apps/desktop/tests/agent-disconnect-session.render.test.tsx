/**
 * Verifies agent connection loss handling and session retention: connection
 * loss is delivered via native event rather than polling, and verified sessions
 * are retained across successful automatic recovery.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { setTimeout as delay } from 'node:timers/promises';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import type { Bridge } from '../src/bridge';
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

test.afterEach(async () => {
  ui.cleanup();
  (await chatMemory()).rememberChatLocation(null);
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

async function chatMemory() {
  return (await vite.ssrLoadModule(
    '/src/location.ts',
  )) as typeof import('../src/location');
}

function deferred<T>(): {
  promise: Promise<T>;
  resolve: (value: T) => void;
} {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((accept) => {
    resolve = accept;
  });
  return { promise, resolve };
}

/** Reference to a remembered chat team, cleared when session state is concealed. */
const REMEMBERED_CHAT = { kind: 'chat', ref: 'team:engineering' } as const;

/**
 * Mounts an App shell with event-driven connection loss handling and waits
 * for initial catalog load. `announce` simulates a dispatched connection loss.
 */
async function connectedShell(overrides: Partial<Bridge> = {}) {
  const { App, FIXTURE, mockBridge } = await modules();
  const listeners = new Set<() => void>();
  let pending: string | null = null;
  let reads = 0;
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    onAgentConnectionLoss: async (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    takeAgentConnectionLoss: async () => {
      reads++;
      const message = pending;
      pending = null;
      return message;
    },
    ...overrides,
  };
  const rendered = ui.render(createElement(App, { bridge }));
  await ui.waitFor(() => {
    const account = document.querySelector<HTMLButtonElement>('button.who');
    assert.ok(account);
    assert.equal(account.disabled, false);
  });
  return {
    rendered,
    listeners,
    reads: () => reads,
    announce: async (message: string) => {
      pending = message;
      await ui.act(async () => {
        for (const listener of [...listeners]) listener();
        await Promise.resolve();
      });
    },
  };
}

test('a loss announced after subscription reaches the shell without polling', async () => {
  const { rendered, listeners, reads, announce } = await connectedShell();
  // Verify that an initial check was performed during mount.
  assert.equal(reads(), 1);
  assert.equal(document.querySelector('.stopwrap'), null);
  // Verify that no periodic polling occurs while connection is idle.
  await delay(1_200);
  assert.equal(reads(), 1);

  await announce('The agent socket closed.');
  const takeover = await rendered.findByRole('alertdialog', {
    name: 'Connection to background service lost',
  });
  assert.match(takeover.textContent ?? '', /agent socket closed/);
  assert.equal(reads(), 2);

  rendered.unmount();
  assert.equal(listeners.size, 0);
});

test('a loss pending prior to subscription is captured during initial check', async () => {
  const { App, FIXTURE, mockBridge } = await modules();
  const subscribed = deferred<void>();
  let reads = 0;
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    native: true,
    onAgentConnectionLoss: async () => {
      await subscribed.promise;
      return () => {};
    },
    takeAgentConnectionLoss: async () => {
      reads++;
      return reads === 1
        ? 'The agent socket closed before the frontend initialized.'
        : null;
    },
  };
  const rendered = ui.render(createElement(App, { bridge }));
  await ui.waitFor(() => assert.ok(document.querySelector('.side.rail')));
  // The error flag is not read until the listener is attached, avoiding dropped events.
  assert.equal(reads, 0);

  await ui.act(async () => {
    subscribed.resolve();
    await Promise.resolve();
  });
  const takeover = await rendered.findByRole('alertdialog', {
    name: 'Connection to background service lost',
  });
  assert.match(takeover.textContent ?? '', /before the frontend initialized/);
  rendered.unmount();
});

test('a disconnect that auto-recovers keeps the chat session', async () => {
  const { rememberChatLocation, rememberedChatRef } = await chatMemory();
  const recovered = deferred<Awaited<ReturnType<Bridge['agentStatus']>>>();
  let automatic = 0;
  const { rendered, announce } = await connectedShell({
    autoRecoverAgent: () => {
      automatic++;
      return recovered.promise;
    },
  });
  rememberChatLocation({ ...REMEMBERED_CHAT });

  await announce('Agent endpoint closed');
  await rendered.findByRole('alertdialog', {
    name: 'Connection to background service lost',
  });
  assert.equal(automatic, 1);
  await ui.act(async () => {
    recovered.resolve({ state: 'ready' });
    await Promise.resolve();
  });
  await ui.waitFor(() =>
    assert.equal(document.querySelector('.stopwrap'), null),
  );

  assert.equal(rememberedChatRef(), REMEMBERED_CHAT.ref);
  rendered.unmount();
});

test('a recovery that returns a different principal discards the session', async () => {
  const { FIXTURE, mockBridge } = await modules();
  const { rememberChatLocation, rememberedChatRef } = await chatMemory();
  const base = mockBridge(FIXTURE);
  const recovered = deferred<Awaited<ReturnType<Bridge['agentStatus']>>>();
  let restarted = false;
  const { rendered, announce } = await connectedShell({
    autoRecoverAgent: () => {
      // The agent comes back holding a different account set.
      restarted = true;
      return recovered.promise;
    },
    listAccounts: async (generation) => {
      const accounts = await base.listAccounts(generation);
      return restarted
        ? accounts.map((account) => ({
            ...account,
            username: `${account.username}.successor`,
          }))
        : accounts;
    },
  });
  rememberChatLocation({ ...REMEMBERED_CHAT });

  await announce('Agent endpoint closed');
  await rendered.findByRole('alertdialog', {
    name: 'Connection to background service lost',
  });
  await ui.act(async () => {
    recovered.resolve({ state: 'ready' });
    await Promise.resolve();
  });
  await ui.waitFor(() =>
    assert.equal(document.querySelector('.stopwrap'), null),
  );

  await ui.waitFor(() => assert.equal(rememberedChatRef(), undefined));
  rendered.unmount();
});

test('a disconnect with no automatic recovery discards the session', async () => {
  const { rememberChatLocation, rememberedChatRef } = await chatMemory();
  const { rendered, announce } = await connectedShell({
    autoRecoverAgent: undefined,
  });
  rememberChatLocation({ ...REMEMBERED_CHAT });

  await announce('Agent endpoint closed');
  await rendered.findByRole('alertdialog', {
    name: 'Connection to background service lost',
  });
  await ui.waitFor(() => assert.equal(rememberedChatRef(), undefined));
  rendered.unmount();
});

test('an agent quarantine discards the session', async () => {
  const { rememberChatLocation, rememberedChatRef } = await chatMemory();
  window.history.replaceState(null, '', '/?state=new');
  const { rendered } = await connectedShell({
    autoRecoverAgent: async () => ({ state: 'ready' }),
    createTextItem: async () => {
      throw {
        code: 'version-mismatch',
        message: 'The agent speaks a newer protocol.',
        retryable: false,
        ambiguous: false,
        fatal: true,
      };
    },
  });
  rememberChatLocation({ ...REMEMBERED_CHAT });

  const site = (await rendered.findByLabelText('Site')) as HTMLInputElement;
  ui.fireEvent.change(site, { target: { value: 'quarantine.example' } });
  await ui.act(async () => {
    rendered.getByRole('button', { name: 'Create item' }).click();
    await Promise.resolve();
  });

  await ui.waitFor(() => assert.equal(rememberedChatRef(), undefined));
  rendered.unmount();
});

test('recovery keeps the shell blocked through partial catalog publication', async () => {
  const { FIXTURE, mockBridge } = await modules();
  const { rememberChatLocation, rememberedChatRef } = await chatMemory();
  const base = mockBridge(FIXTURE);
  const catalogStarted = deferred<void>();
  const completeCatalog = deferred<void>();
  let restarted = false;
  const { rendered, announce } = await connectedShell({
    autoRecoverAgent: async () => {
      restarted = true;
      return { state: 'ready' };
    },
    listCatalog: async (onPartial, force) => {
      const catalog = await base.listCatalog(undefined, force);
      if (restarted) {
        catalog.localMetadata = {
          accounts: (await base.listAccounts(catalog.generation)).map(
            (account) => ({
              ...account,
              username: `${account.username}.successor`,
            }),
          ),
          profiles: await Promise.all(
            (await base.listServers()).map(async (server) => ({
              profile: server.id,
              label: server.label,
              configuredProbe: server.configuredProbe,
              status: await base.describeServerStatus(server.id),
              error: null,
            })),
          ),
        };
        onPartial?.(catalog);
        catalogStarted.resolve();
        await completeCatalog.promise;
      }
      return catalog;
    },
  });
  rememberChatLocation({ ...REMEMBERED_CHAT });

  try {
    await announce('Agent endpoint closed');
    await catalogStarted.promise;
    await ui.waitFor(() =>
      assert.match(
        document.querySelector('button.who')?.textContent ?? '',
        /successor/,
      ),
    );
    assert.ok(
      rendered.getByRole('alertdialog', {
        name: 'Connection to background service lost',
      }),
    );
    assert.equal(rememberedChatRef(), REMEMBERED_CHAT.ref);

    await ui.act(async () => {
      completeCatalog.resolve();
      await Promise.resolve();
    });
    await ui.waitFor(() =>
      assert.equal(document.querySelector('.stopwrap'), null),
    );
    assert.equal(rememberedChatRef(), undefined);
  } finally {
    completeCatalog.resolve();
    rendered.unmount();
  }
});

test('a failed recovery catalog discards the unverified session', async () => {
  const { FIXTURE, mockBridge } = await modules();
  const { rememberChatLocation, rememberedChatRef } = await chatMemory();
  const base = mockBridge(FIXTURE);
  let restarted = false;
  const { rendered, announce } = await connectedShell({
    autoRecoverAgent: async () => {
      restarted = true;
      return { state: 'ready' };
    },
    listCatalog: async (...args) => {
      if (restarted) throw new Error('Catalog unavailable after reconnect');
      return base.listCatalog(...args);
    },
  });
  rememberChatLocation({ ...REMEMBERED_CHAT });
  await announce('Agent endpoint closed');
  await ui.waitFor(() => assert.equal(rememberedChatRef(), undefined));
  rendered.unmount();
});

test('failed connection-loss subscription retains fallback monitoring', async () => {
  let pending: string | null = null;
  let reads = 0;
  const { rendered } = await connectedShell({
    onAgentConnectionLoss: async () => {
      throw new Error('Event registration failed');
    },
    takeAgentConnectionLoss: async () => {
      reads++;
      const result = pending;
      pending = null;
      return result;
    },
  });
  pending = 'Socket closed after registration failed';
  await rendered.findByRole(
    'alertdialog',
    {
      name: 'Connection to background service lost',
    },
    { timeout: 2500 },
  );
  assert.ok(reads >= 2);
  rendered.unmount();
  const stoppedAt = reads;
  await delay(1100);
  assert.equal(reads, stoppedAt);
});
