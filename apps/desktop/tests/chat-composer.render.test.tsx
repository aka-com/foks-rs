import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, useCallback } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import { installDom } from './lib/dom-harness';
import type {
  ChatAction,
  ChatChannel,
  ChatReply,
  ChatScope,
} from '../src/chat-contract';
import type { useChatComposer as Hook } from '../src/chat/use-chat-composer';
import type { useChatHistory as HistoryHook } from '../src/chat/use-chat-history';

installDom({ act: true, timers: true });
let ui: typeof import('@testing-library/react');
let vite: ViteDevServer;
let useChatComposer: typeof Hook;
let useChatHistory: typeof HistoryHook;
test.before(async () => {
  ui = await import('@testing-library/react');
  vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
  ({ useChatComposer } = await vite.ssrLoadModule(
    '/src/chat/use-chat-composer.ts',
  ));
  ({ useChatHistory } = await vite.ssrLoadModule(
    '/src/chat/use-chat-history.ts',
  ));
});
test.afterEach(() => ui.cleanup());
test.after(async () => {
  await vite.close();
});

function deferred() {
  let resolve!: () => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<void>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
const storeId = 'team:eng';
const channelId = 'ab'.repeat(16);
const unavailable = (message: string) => ({
  code: 'deadline-exceeded',
  message,
  retryable: true,
  fatal: false,
  ambiguous: false,
});
async function setup(
  intercept?: (
    action: ChatAction,
    run: () => Promise<ChatReply>,
  ) => Promise<ChatReply>,
) {
  const { ChatInboxProvider, useChatInbox } = (await vite.ssrLoadModule(
    '/src/chat/inbox-provider.tsx',
  )) as typeof import('../src/chat/inbox-provider');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const snapshot = {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server) =>
      server.id === 'acme'
        ? {
            ...server,
            compatibility: { status: 'not-required' as const },
            capabilities: { chat: true },
          }
        : server,
    ),
  };
  const base = mockBridge(snapshot);
  const calls: ChatAction[] = [];
  const bridge = {
    ...base,
    chat: async (store: string, action: ChatAction, view?: string) => {
      calls.push(action);
      const run = () => base.chat(store, action, view);
      return intercept ? intercept(action, run) : run();
    },
  };
  let current!: ReturnType<typeof Hook>;
  let history!: ReturnType<typeof HistoryHook>;
  function Composer({
    channel,
    scope,
  }: {
    channel: ChatChannel;
    scope: ChatScope;
  }) {
    current = useChatComposer(storeId, channel, scope);
    const service = current.service;
    const request = useCallback(
      (action: ChatAction) => service.request(storeId, action),
      [service],
    );
    history = useChatHistory(channel, request);
    return createElement('span', null, current.draft);
  }
  function Conversation() {
    const { snapshot: inbox } = useChatInbox();
    const entry = inbox.get(storeId);
    const channel = entry?.data?.channels.find((item) => item.id === channelId);
    return channel && entry?.scope
      ? createElement(Composer, { channel, scope: entry.scope })
      : null;
  }
  const render = (visible: boolean) =>
    createElement(ChatInboxProvider, {
      bridge,
      snapshot,
      children: visible ? createElement(Conversation) : null,
    });
  const view = ui.render(render(true));
  await ui.waitFor(() => {
    assert.equal(current?.canSend, true);
    assert.equal(history?.busy, false);
    assert.equal(history.error, '');
  });
  return {
    get current() {
      return current;
    },
    get history() {
      return history;
    },
    calls,
    async show(visible: boolean) {
      await ui.act(async () => {
        view.rerender(render(visible));
      });
    },
    async refreshHistory() {
      let pending!: Promise<void>;
      await ui.act(async () => {
        pending = history.load();
      });
      return { pending };
    },
    async send(text: string) {
      await ui.act(async () => {
        current.setDraft(text);
      });
      await ui.act(async () => {
        current.send();
      });
    },
  };
}

function assertSent(current: ReturnType<typeof Hook>, count: number) {
  assert.equal(current.messages.length, count);
  assert.ok(
    current.messages.every(
      (message) => message.phase === 'sent' && !message.running,
    ),
  );
  assert.equal(current.canSend, true);
  assert.equal(current.sendError, '');
}

test('production composer import aliases the canonical application-service hook', async () => {
  const { useMessageComposer } = await vite.ssrLoadModule(
    '/src/chat/use-message-composer.ts',
  );
  assert.equal(useMessageComposer, useChatComposer);
});

test('confirmed delivery releases the composer before delayed history completes', async () => {
  const history = deferred();
  let delayed = false;
  let refresh: Promise<void> | undefined;
  const h = await setup(async (action, run) => {
    if (action.action === 'history' && delayed) await history.promise;
    const reply = await run();
    if (action.action === 'attempt' && !refresh) refresh = load();
    return reply;
  });
  const load = (): Promise<void> => h.history.load();
  delayed = true;
  try {
    await h.send('first');
    await ui.waitFor(() => assertSent(h.current, 1));
    assert.equal(h.history.busy, true);
    assert.ok(h.calls.some((action) => action.action === 'history'));
    await h.send('second');
    assert.equal(h.current.messages.length, 2);
    assert.equal(h.current.messages[1].text, 'second');
    assert.equal(h.current.draft, '');
    assert.equal(h.current.messages[0].running, false);
    await ui.act(async () => {
      history.resolve();
      await refresh;
    });
    await ui.waitFor(() => assertSent(h.current, 2));
    assert.equal(
      h.calls.filter((action) => action.action === 'attempt').length,
      2,
    );
    const ids = h.calls
      .filter((action) => action.action === 'prepare-message')
      .map((action) => action.submission);
    assert.equal(new Set(ids).size, 2);
  } finally {
    await ui.act(async () => {
      history.resolve();
      await refresh;
    });
  }
});

test('pending-view refresh is not a prerequisite to attempting prepared delivery', async () => {
  const pendingView = deferred();
  let refresh: Promise<void> | undefined;
  const h = await setup(async (action, run) => {
    if (action.action === 'pending') await pendingView.promise;
    const reply = await run();
    if (action.action === 'attempt') refresh = refreshPending();
    return reply;
  });
  const refreshPending = (): Promise<void> => h.current.service.refresh(storeId);
  try {
    await h.send('send before optional refresh');
    await ui.waitFor(() => assertSent(h.current, 1));
    assert.ok(h.calls.some((action) => action.action === 'pending'));
    assert.equal(
      h.calls.filter((action) => action.action === 'attempt').length,
      1,
    );
  } finally {
    await ui.act(async () => {
      pendingView.resolve();
      await refresh;
    });
  }
});

test('display refresh failure is reported separately and never retries delivery', async () => {
  let failHistory = false;
  const h = await setup(async (action, run) => {
    if (action.action === 'history' && failHistory)
      throw unavailable('History unavailable');
    return run();
  });
  await h.send('confirmed once');
  await ui.waitFor(() => assertSent(h.current, 1));
  failHistory = true;
  const refresh = await h.refreshHistory();
  await refresh.pending;
  assert.equal(h.history.error, 'History unavailable');
  assertSent(h.current, 1);
  assert.equal(h.current.messages[0].error, '');
  assert.equal(
    h.calls.filter((action) => action.action === 'attempt').length,
    1,
  );
});

test('late refresh failures cannot overwrite a later send state', async () => {
  const old = deferred();
  let delayed = false;
  const h = await setup(async (action, run) => {
    if (action.action === 'history' && delayed) await old.promise;
    return run();
  });
  await h.send('first');
  await ui.waitFor(() => assertSent(h.current, 1));
  delayed = true;
  const refresh = await h.refreshHistory();
  try {
    await h.send('second');
    assert.equal(h.current.messages[1].text, 'second');
    assert.equal(h.current.sendError, '');
    await ui.act(async () => {
      old.reject(unavailable('old refresh failed'));
      await refresh.pending;
    });
    assert.equal(h.history.error, 'old refresh failed');
    await ui.waitFor(() => assertSent(h.current, 2));
    assert.ok(h.current.messages.every((message) => message.error === ''));
    assert.equal(
      h.calls.filter((action) => action.action === 'attempt').length,
      2,
    );
  } finally {
    await ui.act(async () => {
      old.resolve();
      await refresh.pending;
    });
  }
});

test('ambiguous delivery can finish the composer while a read-only status check waits', async () => {
  const status = deferred();
  const h = await setup(async (action, run) => {
    if (action.action === 'attempt')
      throw {
        code: 'ambiguous',
        message: 'Delivery unknown',
        ambiguous: true,
        retryable: false,
        fatal: false,
      };
    if (action.action === 'status') await status.promise;
    return run();
  });
  try {
    await h.send('may have been delivered');
    await ui.waitFor(() => {
      assert.equal(h.current.messages[0]?.running, false);
      assert.ok(h.calls.some((action) => action.action === 'status'));
    });
    assert.equal(h.current.canSend, true);
    assert.equal(h.current.sendError, '');
    assert.equal(h.current.messages[0].error, 'Delivery unknown');
    assert.equal(h.current.messages[0].phase, 'unconfirmed');
    assert.equal(
      h.calls.filter((action) => action.action === 'attempt').length,
      1,
    );
    await ui.act(async () => {
      h.current.setDraft('next draft');
    });
    assert.equal(h.current.draft, 'next draft');
  } finally {
    await ui.act(async () => {
      status.resolve();
    });
  }
});

test('an older delivery error belongs to its message, not a newer send', async () => {
  const old = deferred();
  let attempts = 0;
  const h = await setup(async (action, run) => {
    if (action.action === 'attempt' && ++attempts === 1) {
      await old.promise;
      throw unavailable('older delivery failed');
    }
    return run();
  });
  try {
    await h.send('first');
    await ui.waitFor(() => assert.equal(h.current.canSend, true));
    await h.send('second');
    assert.equal(h.current.messages[1].text, 'second');
    await ui.act(async () => {
      old.resolve();
    });
    await ui.waitFor(() => {
      assert.equal(h.current.messages[0].running, false);
      assert.equal(h.current.messages[1].phase, 'sent');
    });
    assert.equal(h.current.messages[0].error, 'older delivery failed');
    assert.equal(h.current.messages[1].error, '');
    assert.equal(h.current.messages[1].phase, 'sent');
    assert.equal(h.current.sendError, '');
    assert.equal(h.current.canSend, true);
    assert.equal(attempts, 2);
  } finally {
    await ui.act(async () => {
      old.resolve();
    });
  }
});

test('the application service completes preparation after the composer unmounts', async () => {
  const preparation = deferred();
  const h = await setup(async (action, run) => {
    if (action.action === 'prepare-message') await preparation.promise;
    return run();
  });
  try {
    await h.send('outlives this view');
    const service = h.current.service;
    await ui.act(async () => {
      h.current.setDraft('next draft');
    });
    await h.show(false);
    await ui.act(async () => {
      preparation.resolve();
    });
    await ui.waitFor(() =>
      assert.equal(service.messages(storeId, channelId)[0].phase, 'sent'),
    );
    await h.show(true);
    assert.equal(h.current.service, service);
    assert.equal(h.current.draft, 'next draft');
    assertSent(h.current, 1);
    assert.equal(
      h.calls.filter((action) => action.action === 'attempt').length,
      1,
    );
  } finally {
    await ui.act(async () => {
      preparation.resolve();
    });
  }
});
