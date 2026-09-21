import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, useCallback, useState } from 'react';
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
    run: (action?: ChatAction) => Promise<ChatReply>,
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
            services: { chat: true },
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
      const run = (input = action) => base.chat(store, input, view);
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

function assertSent(
  current: ReturnType<typeof Hook>,
  count: number,
  canSend = true,
) {
  assert.equal(current.messages.length, count);
  assert.ok(
    current.messages.every(
      (message) => message.phase === 'sent' && !message.running,
    ),
  );
  assert.equal(current.canSend, canSend);
  assert.equal(current.sendError, '');
}

test('production composer import aliases the canonical application-service hook', async () => {
  const { useMessageComposer } = await vite.ssrLoadModule(
    '/src/chat/use-message-composer.ts',
  );
  assert.equal(useMessageComposer, useChatComposer);
});

test('confirmed delivery finishes while delayed history holds intent cleanup', async () => {
  const history = deferred();
  let delayed = false;
  let refresh: Promise<void> | undefined;
  const h = await setup(async (action, run) => {
    if (action.action === 'history' && delayed) await history.promise;
    const reply = await run();
    if (action.action === 'submit-message' && !refresh) refresh = load();
    return reply;
  });
  const load = (): Promise<void> => h.history.load();
  delayed = true;
  try {
    await h.send('first');
    await ui.waitFor(() => assertSent(h.current, 1));
    assert.equal(h.history.busy, true);
    assert.ok(h.calls.some((action) => action.action === 'history'));
    await ui.act(async () => h.current.setDraft('second'));
    assert.equal(h.current.messages.length, 1);
    assert.equal(h.current.draft, 'second');
    // A held history refresh is not something the composer waits on.
    assert.equal(h.current.canSend, true);
    assert.equal(h.current.messages[0].running, false);
    await ui.act(async () => {
      history.resolve();
      await refresh;
    });
    await ui.waitFor(() => assertSent(h.current, 1));
    await h.send('second');
    await ui.waitFor(() => assertSent(h.current, 2));
    assert.equal(
      h.calls.filter((action) => action.action === 'submit-message').length,
      2,
    );
    const ids = h.calls
      .filter((action) => action.action === 'submit-message')
      .map((action) => action.submission);
    assert.equal(new Set(ids).size, 2);
  } finally {
    await ui.act(async () => {
      history.resolve();
      await refresh;
    });
  }
});

test('pending-view refresh is not a prerequisite to submitting delivery', async () => {
  const pendingView = deferred();
  let refresh: Promise<void> | undefined;
  const h = await setup(async (action, run) => {
    if (action.action === 'pending') await pendingView.promise;
    const reply = await run();
    if (action.action === 'submit-message') refresh = refreshPending();
    return reply;
  });
  const refreshPending = (): Promise<void> =>
    h.current.service.refresh(storeId);
  try {
    await h.send('send before optional refresh');
    await ui.waitFor(() => assertSent(h.current, 1));
    assert.ok(h.calls.some((action) => action.action === 'pending'));
    assert.equal(
      h.calls.filter((action) => action.action === 'submit-message').length,
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
    h.calls.filter((action) => action.action === 'submit-message').length,
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
      h.calls.filter((action) => action.action === 'submit-message').length,
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
    if (action.action === 'submit-message')
      return run({ ...action, action: 'prepare-message' });
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
    assert.equal(
      h.calls.filter((action) => action.action === 'submit-message').length,
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
    if (action.action === 'submit-message')
      return run({ ...action, action: 'prepare-message' });
    if (action.action === 'attempt' && ++attempts === 1) {
      await old.promise;
      throw unavailable('older delivery failed');
    }
    return run();
  });
  try {
    await h.send('first');
    await ui.waitFor(() => assert.equal(attempts, 1));
    // The attempt is still out; the composer is open because another send
    // would queue behind this message rather than wait for it.
    assert.equal(h.current.canSend, true);
    await ui.act(async () => h.current.setDraft('second'));
    assert.equal(h.current.draft, 'second');
    await ui.act(async () => {
      old.resolve();
    });
    await ui.waitFor(() =>
      assert.equal(h.current.messages[0].error, 'older delivery failed'),
    );
    await h.send('second');
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
    if (action.action === 'submit-message') await preparation.promise;
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
      h.calls.filter((action) => action.action === 'submit-message').length,
      1,
    );
  } finally {
    await ui.act(async () => {
      preparation.resolve();
    });
  }
});

test('history revisions append incrementally, retain older rows, and replace gaps and resets', async () => {
  const { conversationResult } = (await vite.ssrLoadModule(
    '/src/chat/conversation-model.ts',
  )) as typeof import('../src/chat/conversation-model');
  const { eventFromReply } = (await vite.ssrLoadModule(
    '/src/chat/conversation-events.ts',
  )) as typeof import('../src/chat/conversation-events');
  const channel: ChatChannel = {
    id: channelId,
    name: '',
    description: null,
    admin: false,
    readable: true,
    writable: true,
    read_role: 'member',
    write_role: 'member',
  };
  const calls: ChatAction[] = [];
  let rows = [2, 1];
  let gap = false;
  const request = async (action: ChatAction): Promise<ChatReply> => {
    assert.equal(action.action, 'history');
    calls.push(action);
    const selected = rows.filter(
      (n) =>
        (action.after === undefined || n > Number(action.after)) &&
        (action.before === null || n < Number(action.before)),
    );
    return {
      scope: {} as ChatScope,
      result: {
        kind: 'history',
        channel: action.channel,
        messages: selected.map((n) => ({
          id: `m${n}`,
          sequence: String(n),
          sender: null,
          send_time: '1',
          insert_time: '1',
          content: { kind: 'unsupported' },
        })),
        before:
          selected.length && Math.min(...selected) > 1
            ? String(Math.min(...selected))
            : null,
        missing_predecessors: [],
        ...(action.after === undefined ? {} : { gap }),
      },
    };
  };
  let current!: ReturnType<typeof HistoryHook>;
  function History({
    revision,
    incremental = true,
  }: {
    revision: number;
    incremental?: boolean;
  }) {
    const [held, setHeld] = useState<
      import('../src/chat/conversation-model').HistoryWindow | null
    >(null);
    const accept = useCallback(
      (
        page: Extract<
          import('../src/chat-contract').ChatResult,
          { kind: 'history' }
        >,
        before: string | null,
        replace?: boolean,
      ) => {
        setHeld(
          (previous) =>
            conversationResult(
              { operations: [], history: replace ? null : previous },
              eventFromReply(
                { action: 'history', channel: channelId, before },
                page,
              ),
            ).history,
        );
      },
      [],
    );
    current = useChatHistory(
      channel,
      request,
      revision,
      accept,
      held,
      undefined,
      undefined,
      incremental,
    );
    return null;
  }
  const view = ui.render(createElement(History, { revision: 0 }));
  await ui.waitFor(() => assert.equal(current.messages.length, 2));
  assert.deepEqual(calls[0], {
    action: 'history',
    channel: channelId,
    before: null,
  });
  rows = [3, 2, 1];
  view.rerender(createElement(History, { revision: 1 }));
  await ui.waitFor(() => assert.equal(current.messages.length, 3));
  assert.deepEqual(calls.at(-1), {
    action: 'history',
    channel: channelId,
    before: null,
    after: '2',
  });
  assert.equal(current.before, null);
  // An empty delta retains the accepted window.
  view.rerender(createElement(History, { revision: 2 }));
  await ui.waitFor(() => assert.equal(calls.length, 3));
  assert.deepEqual(
    current.messages.map((m) => m.sequence),
    ['1', '2', '3'],
  );
  rows = [100, 99];
  gap = true;
  view.rerender(createElement(History, { revision: 3 }));
  await ui.waitFor(() =>
    assert.deepEqual(
      current.messages.map((m) => m.sequence),
      ['99', '100'],
    ),
  );
  assert.deepEqual(
    calls.slice(-2).map((a) => (a.action === 'history' ? a.after : undefined)),
    ['3', undefined],
  );
  rows = [1];
  view.rerender(createElement(History, { revision: 4 }));
  await ui.waitFor(() =>
    assert.deepEqual(
      current.messages.map((m) => m.sequence),
      ['1'],
    ),
  );
  // Older agents never receive the unknown after field.
  view.rerender(createElement(History, { revision: 4, incremental: false }));
  await ui.waitFor(() => assert.equal(current.busy, false));
  const count = calls.length;
  rows = [2, 1];
  view.rerender(createElement(History, { revision: 5, incremental: false }));
  await ui.waitFor(() => assert.equal(calls.length, count + 1));
  assert.equal('after' in calls.at(-1)!, false);
  await ui.act(async () => {
    await current.load('2');
  });
  assert.deepEqual(calls.at(-1), {
    action: 'history',
    channel: channelId,
    before: '2',
  });
});

test('changing channels ignores the old in-flight page and starts a full load', async () => {
  const first = deferred();
  const calls: ChatAction[] = [];
  const accepted: string[] = [];
  const accept = (
    page: Extract<
      import('../src/chat-contract').ChatResult,
      { kind: 'history' }
    >,
  ) => {
    accepted.push(page.channel);
  };
  const request = async (action: ChatAction): Promise<ChatReply> => {
    assert.equal(action.action, 'history');
    calls.push(action);
    if (action.channel === 'a') await first.promise;
    return {
      scope: {} as ChatScope,
      result: {
        kind: 'history',
        channel: action.channel,
        messages: [],
        before: null,
        missing_predecessors: [],
      },
    };
  };
  const channel: ChatChannel = {
    id: 'a',
    name: '',
    description: null,
    admin: false,
    readable: true,
    writable: true,
    read_role: 'member',
    write_role: 'member',
  };
  function History({ id }: { id: string }) {
    useChatHistory(
      { ...channel, id },
      request,
      0,
      accept,
      null,
      undefined,
      undefined,
      true,
    );
    return null;
  }
  const view = ui.render(createElement(History, { id: 'a' }));
  await ui.waitFor(() => assert.equal(calls.length, 1));
  view.rerender(createElement(History, { id: 'b' }));
  await ui.waitFor(() => assert.deepEqual(accepted, ['b']));
  await ui.act(async () => {
    first.resolve();
    await first.promise;
  });
  assert.deepEqual(accepted, ['b']);
  assert.deepEqual(calls, [
    { action: 'history', channel: 'a', before: null },
    { action: 'history', channel: 'b', before: null },
  ]);
});

test('a tail costs one read per arrival, and a revision that moved no position costs none', async () => {
  const { conversationResult } = (await vite.ssrLoadModule(
    '/src/chat/conversation-model.ts',
  )) as typeof import('../src/chat/conversation-model');
  const { eventFromReply } = (await vite.ssrLoadModule(
    '/src/chat/conversation-events.ts',
  )) as typeof import('../src/chat/conversation-events');
  const channel: ChatChannel = {
    id: channelId,
    name: '',
    description: null,
    admin: false,
    readable: true,
    writable: true,
    read_role: 'member',
    write_role: 'member',
  };
  const calls: ChatAction[] = [];
  let rows = [2, 1];
  let gap = false;
  let missing: string[] = [];
  const request = async (action: ChatAction): Promise<ChatReply> => {
    assert.equal(action.action, 'history');
    calls.push(action);
    const selected = rows.filter(
      (n) =>
        (action.after === undefined || n > Number(action.after)) &&
        (action.before === null || n < Number(action.before)),
    );
    return {
      scope: {} as ChatScope,
      result: {
        kind: 'history',
        channel: action.channel,
        messages: selected.map((n) => ({
          id: `m${n}`,
          sequence: String(n),
          sender: null,
          send_time: '1',
          insert_time: '1',
          content: { kind: 'unsupported' },
        })),
        before:
          selected.length && Math.min(...selected) > 1
            ? String(Math.min(...selected))
            : null,
        missing_predecessors: action.after === undefined ? [] : missing,
        ...(action.after === undefined ? {} : { gap }),
      },
    };
  };
  let current!: ReturnType<typeof HistoryHook>;
  function History({
    revision,
    position,
  }: {
    revision: number;
    position: string | null;
  }) {
    const [held, setHeld] = useState<
      import('../src/chat/conversation-model').HistoryWindow | null
    >(null);
    const accept = useCallback(
      (
        page: Extract<
          import('../src/chat-contract').ChatResult,
          { kind: 'history' }
        >,
        before: string | null,
        replace?: boolean,
      ) => {
        setHeld(
          (previous) =>
            conversationResult(
              { operations: [], history: replace ? null : previous },
              eventFromReply(
                { action: 'history', channel: channelId, before },
                page,
              ),
            ).history,
        );
      },
      [],
    );
    current = useChatHistory(
      channel,
      request,
      revision,
      accept,
      held,
      undefined,
      undefined,
      true,
      position,
    );
    return null;
  }
  const view = ui.render(
    createElement(History, { revision: 0, position: '2' }),
  );
  await ui.waitFor(() => assert.equal(current.messages.length, 2));
  assert.equal(calls.length, 1);
  // A read on another device advances the read pointer and drops the unread
  // count by the same amount, so the published position stands still.
  view.rerender(createElement(History, { revision: 1, position: '2' }));
  await ui.waitFor(() => assert.equal(current.busy, false));
  assert.equal(calls.length, 1);
  // One arrival costs one tail read holding one row, not a page of fifty.
  rows = [3, 2, 1];
  view.rerender(createElement(History, { revision: 2, position: '3' }));
  await ui.waitFor(() => assert.equal(current.messages.length, 3));
  assert.equal(calls.length, 2);
  assert.deepEqual(calls.at(-1), {
    action: 'history',
    channel: channelId,
    before: null,
    after: '2',
  });
  assert.equal(current.before, null);
  assert.equal(current.missing, false);
  // A predecessor the tail could not verify is incomplete evidence, and the
  // conversation says so rather than presenting the row as checked.
  rows = [4, 3, 2, 1];
  missing = ['3'];
  view.rerender(createElement(History, { revision: 3, position: '4' }));
  await ui.waitFor(() => assert.equal(current.messages.length, 4));
  assert.equal(current.missing, true);
  missing = [];
  // A tail that cannot cover the distance reports a gap, and the fallback
  // full page is not itself gated by the position it is recovering from.
  rows = [100, 99];
  gap = true;
  view.rerender(createElement(History, { revision: 4, position: '100' }));
  await ui.waitFor(() =>
    assert.deepEqual(
      current.messages.map((m) => m.sequence),
      ['99', '100'],
    ),
  );
  assert.deepEqual(
    calls.slice(-2).map((a) => (a.action === 'history' ? a.after : undefined)),
    ['4', undefined],
  );
  // An unknown position never withholds a read.
  gap = false;
  rows = [101, 100, 99];
  const issued = calls.length;
  view.rerender(createElement(History, { revision: 5, position: null }));
  await ui.waitFor(() =>
    assert.deepEqual(
      current.messages.map((m) => m.sequence),
      ['99', '100', '101'],
    ),
  );
  assert.equal(calls.length - issued, 1);
  assert.deepEqual(calls.at(-1), {
    action: 'history',
    channel: channelId,
    before: null,
    after: '100',
  });
});
