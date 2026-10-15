import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, useCallback, useState } from 'react';
import { installDom, settle } from './lib/dom-harness';
import type {
  ChatAction,
  ChatChannel,
  ChatReply,
  ChatScope,
} from '../src/chat-contract';
import type {
  useChatComposer as Hook,
  useChatDraft as DraftHook,
} from '../src/chat/use-chat-composer';
import type { useChatHistory as HistoryHook } from '../src/chat/use-chat-history';
import { phaseOf, running } from '../src/chat/send/outgoing';

installDom({ act: true, timers: true });
let ui: typeof import('@testing-library/react');
let useChatComposer: typeof Hook;
let useChatDraft: typeof DraftHook;
let useChatHistory: typeof HistoryHook;
test.before(async () => {
  ui = await import('@testing-library/react');
  ({ useChatComposer, useChatDraft } =
    await import('../src/chat/use-chat-composer'));
  ({ useChatHistory } = await import('../src/chat/use-chat-history'));
});
test.afterEach(() => ui.cleanup());

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
  const { ChatInboxProvider, useChatInbox } =
    await import('../src/chat/inbox-provider');
  const { FIXTURE } = await import('../src/fixture');
  const { mockBridge } = await import('../src/mock-bridge');
  const snapshot = {
    ...FIXTURE,
    servers: FIXTURE.servers.map((server) =>
      server.profileName === 'acme'
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
    chat: async (store: string, action: ChatAction, view: string) => {
      calls.push(action);
      const run = (input = action) => base.chat(store, input, view);
      return intercept ? intercept(action, run) : run();
    },
  };
  let current!: ReturnType<typeof Hook>;
  let draft!: ReturnType<typeof DraftHook>;
  let history!: ReturnType<typeof HistoryHook>;
  // How often each layer has rendered: the thread's send state, which also
  // holds the message list, and the field that reads the draft.
  const renders = { thread: 0, draft: 0 };
  function Draft({ channel }: { channel: ChatChannel }) {
    renders.draft++;
    draft = useChatDraft(storeId, channel.id);
    return createElement('span', null, draft.draft);
  }
  function Composer({
    channel,
    scope,
  }: {
    channel: ChatChannel;
    scope: ChatScope;
  }) {
    renders.thread++;
    current = useChatComposer(storeId, channel, scope);
    const service = current.service;
    const request = useCallback(
      (action: ChatAction) => service.request(storeId, action),
      [service],
    );
    history = useChatHistory({ channel, request });
    return createElement(Draft, { channel });
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
      return { ...current, ...draft };
    },
    renders,
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
  current: Pick<ReturnType<typeof Hook>, 'messages' | 'canSend' | 'sendError'>,
  count: number,
  canSend = true,
) {
  assert.equal(current.messages.length, count);
  assert.ok(
    current.messages.every(
      (message) => phaseOf(message) === 'sent' && !running(message),
    ),
  );
  assert.equal(current.canSend, canSend);
  assert.equal(current.sendError, '');
}

test('typing re-renders only the draft reader, not the thread that draws the message list', async () => {
  const h = await setup();
  let thread = 0;
  let draft = 0;
  // A synchronous act renders what the change updates before it returns, and
  // lets no background reply in, so the counts are the change's alone.
  for (const text of ['h', 'he', 'hel', 'he']) {
    const before = { ...h.renders };
    ui.act(() => h.current.setDraft(text));
    thread += h.renders.thread - before.thread;
    draft += h.renders.draft - before.draft;
    await ui.act(async () => {});
  }
  assert.equal(h.current.draft, 'he');
  assert.equal(h.current.draftBytes, 2);
  assert.equal(draft, 4);
  // The thread subscribes to the send service's revision; a draft change is
  // not a publication, so it does not render again.
  assert.equal(thread, 0);
  // Sending still reads the current draft.
  await ui.act(async () => {
    assert.equal(h.current.send(), true);
  });
  await ui.waitFor(() => assertSent(h.current, 1));
  assert.equal(h.current.draft, '');
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
    assert.equal(running(h.current.messages[0]), false);
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
      assert.equal(
        h.current.messages[0] && running(h.current.messages[0]),
        false,
      );
      assert.ok(h.calls.some((action) => action.action === 'status'));
    });
    assert.equal(h.current.canSend, true);
    assert.equal(h.current.sendError, '');
    assert.equal(h.current.messages[0].error, 'Delivery unknown');
    assert.equal(phaseOf(h.current.messages[0]), 'unconfirmed');
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
      assert.equal(running(h.current.messages[0]), false);
      assert.equal(phaseOf(h.current.messages[1]), 'sent');
    });
    assert.equal(h.current.messages[0].error, 'older delivery failed');
    assert.equal(h.current.messages[1].error, '');
    assert.equal(phaseOf(h.current.messages[1]), 'sent');
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
      assert.equal(phaseOf(service.messages(storeId, channelId)[0]), 'sent'),
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
  const { mergeHistoryPage } = await import('../src/chat/conversation-model');
  const { historyEvent } = await import('../src/chat/conversation-events');
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
        placement?: import('../src/chat/conversation-model').HistoryPlacement,
      ) => {
        setHeld((previous) =>
          mergeHistoryPage(
            placement?.replace ? null : previous,
            historyEvent(page, before),
          ),
        );
      },
      [],
    );
    current = useChatHistory({
      channel,
      request,
      revision,
      history: held,
      incremental,
      onAccepted: accept,
    });
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
    useChatHistory({
      channel: { ...channel, id },
      request,
      incremental: true,
      onAccepted: accept,
    });
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

test('inline history callbacks neither reload the window nor go stale', async () => {
  const { mergeHistoryPage } = await import('../src/chat/conversation-model');
  const { historyEvent } = await import('../src/chat/conversation-events');
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
  let pending: Promise<void> | null = null;
  const request = async (action: ChatAction): Promise<ChatReply> => {
    assert.equal(action.action, 'history');
    calls.push(action);
    if (pending) await pending;
    const selected = rows.filter(
      (n) => action.after === undefined || n > Number(action.after),
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
        before: null,
        missing_predecessors: [],
        ...(action.after === undefined ? {} : { gap: false }),
      },
    };
  };
  const accepted: string[] = [];
  const loading: string[] = [];
  let current!: ReturnType<typeof HistoryHook>;
  function History({ revision, label }: { revision: number; label: string }) {
    const [held, setHeld] = useState<
      import('../src/chat/conversation-model').HistoryWindow | null
    >(null);
    // Every render passes new callbacks, as a caller that does not memoize
    // them would.
    current = useChatHistory({
      channel,
      request,
      revision,
      history: held,
      incremental: true,
      onAccepted: (page, before, placement) => {
        accepted.push(label);
        setHeld((previous) =>
          mergeHistoryPage(
            placement?.replace ? null : previous,
            historyEvent(page, before),
          ),
        );
      },
      onLoading: (load) => loading.push(`${label}:${load}`),
    });
    return null;
  }
  const view = ui.render(
    createElement(History, { revision: 0, label: 'first' }),
  );
  await ui.waitFor(() => assert.equal(current.messages.length, 2));
  view.rerender(createElement(History, { revision: 0, label: 'second' }));
  await ui.act(settle);
  assert.equal(calls.length, 1);
  // A later read reports to the callbacks of the latest render, including
  // a render that happens while the read is in flight.
  rows = [3, 2, 1];
  const reply = deferred();
  pending = reply.promise;
  view.rerender(createElement(History, { revision: 1, label: 'second' }));
  await ui.waitFor(() => assert.equal(calls.length, 2));
  view.rerender(createElement(History, { revision: 1, label: 'third' }));
  await ui.act(async () => {
    reply.resolve();
    await reply.promise;
  });
  await ui.waitFor(() => assert.equal(current.messages.length, 3));
  assert.deepEqual(accepted, ['first', 'third']);
  assert.deepEqual(loading, ['first:tail', 'second:incremental']);
  assert.deepEqual(calls.at(-1), {
    action: 'history',
    channel: channelId,
    before: null,
    after: '2',
  });
  assert.equal(calls.length, 2);
});

test('a new request owner reads the window again and drops the old reply', async () => {
  const first = deferred();
  const page = (id: string): ChatReply => ({
    scope: {} as ChatScope,
    result: {
      kind: 'history',
      channel: channelId,
      messages: [
        {
          id,
          sequence: '1',
          sender: null,
          send_time: '1',
          insert_time: '1',
          content: { kind: 'unsupported' },
        },
      ],
      before: null,
      missing_predecessors: [],
    },
  });
  const oldCalls: ChatAction[] = [];
  const newCalls: ChatAction[] = [];
  const oldRequest = async (action: ChatAction): Promise<ChatReply> => {
    oldCalls.push(action);
    await first.promise;
    return page('old');
  };
  const newRequest = async (action: ChatAction): Promise<ChatReply> => {
    newCalls.push(action);
    return page('new');
  };
  const accepted: string[] = [];
  const accept = (
    result: Extract<
      import('../src/chat-contract').ChatResult,
      { kind: 'history' }
    >,
  ) => {
    accepted.push(result.messages[0].id);
  };
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
  function History({
    request,
  }: {
    request: (action: ChatAction) => Promise<ChatReply>;
  }) {
    useChatHistory({ channel, request, onAccepted: accept });
    return null;
  }
  const view = ui.render(createElement(History, { request: oldRequest }));
  await ui.waitFor(() => assert.equal(oldCalls.length, 1));
  view.rerender(createElement(History, { request: newRequest }));
  await ui.waitFor(() => assert.deepEqual(accepted, ['new']));
  await ui.act(async () => {
    first.resolve();
    await first.promise;
  });
  assert.deepEqual(accepted, ['new']);
  assert.deepEqual(newCalls, [
    { action: 'history', channel: channelId, before: null },
  ]);
});

test('a tail costs one read per arrival, and a revision that moved no position costs none', async () => {
  const { mergeHistoryPage } = await import('../src/chat/conversation-model');
  const { historyEvent } = await import('../src/chat/conversation-events');
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
        placement?: import('../src/chat/conversation-model').HistoryPlacement,
      ) => {
        setHeld((previous) =>
          mergeHistoryPage(
            placement?.replace ? null : previous,
            historyEvent(page, before),
          ),
        );
      },
      [],
    );
    current = useChatHistory({
      channel,
      request,
      revision,
      history: held,
      position,
      incremental: true,
      onAccepted: accept,
    });
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

test('an unread channel opens at its first unread row and pages forward while detached', async () => {
  const { mergeHistoryPage } = await import('../src/chat/conversation-model');
  const { historyEvent } = await import('../src/chat/conversation-events');
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
  let head = 120;
  // Rows too large for one page make the agent halve the width, keeping the
  // requested end.
  let narrow = false;
  // The agent's semantics: `before` reads the fifty sequences below it, and
  // `after` reads up to fifty rows past it, reporting a gap when the head
  // is further away or below the cursor.
  const request = async (action: ChatAction): Promise<ChatReply> => {
    assert.equal(action.action, 'history');
    calls.push(action);
    const rows: number[] = [];
    let gap: boolean | undefined;
    if (action.after !== undefined) {
      const after = Number(action.after);
      gap = head < after || head - after > 50;
      for (let n = Math.max(after + 1, head - 49); n <= head; n++) rows.push(n);
    } else {
      const end = action.before === null ? head : Number(action.before) - 1;
      for (
        let n = Math.max(1, end - (narrow ? 24 : 49));
        n <= Math.min(end, head);
        n++
      )
        rows.push(n);
    }
    return {
      scope: {} as ChatScope,
      result: {
        kind: 'history',
        channel: action.channel,
        messages: rows.map((n) => ({
          id: `m${n}`,
          sequence: String(n),
          sender: null,
          send_time: '1',
          insert_time: '1',
          content: { kind: 'unsupported' },
        })),
        before: rows.length && rows[0] > 1 ? String(rows[0]) : null,
        missing_predecessors: [],
        ...(gap === undefined ? {} : { gap }),
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
        placement: import('../src/chat/conversation-model').HistoryPlacement = {},
      ) => {
        setHeld((previous) =>
          mergeHistoryPage(
            placement.replace ? null : previous,
            historyEvent(page, before, placement),
          ),
        );
      },
      [],
    );
    current = useChatHistory({
      channel,
      request,
      revision,
      history: held,
      position,
      anchor: '60',
      incremental: true,
      onAccepted: accept,
    });
    return null;
  }
  const sequences = () => current.messages.map((m) => Number(m.sequence));
  const range = (from: number, to: number) =>
    Array.from({ length: to - from + 1 }, (_, i) => from + i);
  const view = ui.render(
    createElement(History, { revision: 0, position: '120' }),
  );
  // Read through 60: ten read rows, then forty unread ones.
  await ui.waitFor(() => assert.equal(current.messages.length, 50));
  assert.deepEqual(calls, [
    { action: 'history', channel: channelId, before: '101' },
  ]);
  assert.deepEqual(sequences(), range(51, 100));
  assert.equal(current.detached, true);
  assert.equal(current.before, '51');
  // An arrival past a detached window reads nothing: an `after` read from
  // its tail would report a gap for the distance alone.
  head = 121;
  view.rerender(createElement(History, { revision: 1, position: '121' }));
  await ui.waitFor(() => assert.equal(current.busy, false));
  assert.equal(calls.length, 1);
  // Paging forward is an explicit range read past the tail, and the page
  // that reaches the head attaches the window again.
  await ui.act(async () => {
    await current.loadNewer();
  });
  assert.deepEqual(calls.at(-1), {
    action: 'history',
    channel: channelId,
    before: '151',
  });
  assert.deepEqual(sequences(), range(51, 121));
  assert.equal(current.detached, false);
  assert.equal(current.before, '51');
  // Attached, arrivals are incremental again.
  head = 122;
  view.rerender(createElement(History, { revision: 2, position: '122' }));
  await ui.waitFor(() => assert.equal(current.messages.length, 72));
  assert.deepEqual(calls.at(-1), {
    action: 'history',
    channel: channelId,
    before: null,
    after: '121',
  });
  // Reopening at the read pointer detaches the window again, and a head the
  // inbox publishes below its tail is a rollback: the gap the `after` read
  // reports still replaces the window with the newest page.
  await ui.act(async () => {
    await current.reopen('60');
  });
  assert.deepEqual(sequences(), range(51, 100));
  assert.equal(current.detached, true);
  head = 40;
  view.rerender(createElement(History, { revision: 3, position: '40' }));
  await ui.waitFor(() => assert.deepEqual(sequences(), range(1, 40)));
  assert.deepEqual(
    calls.slice(-2).map((a) => (a.action === 'history' ? a.after : undefined)),
    ['100', undefined],
  );
  assert.equal(current.detached, false);
  // Jump to latest replaces a detached window with the newest page.
  head = 140;
  view.rerender(createElement(History, { revision: 4, position: '140' }));
  await ui.waitFor(() =>
    assert.equal(current.messages.at(-1)?.sequence, '140'),
  );
  await ui.act(async () => {
    await current.reopen('60');
  });
  assert.equal(current.detached, true);
  await ui.act(async () => {
    await current.loadLatest();
  });
  assert.deepEqual(sequences(), range(91, 140));
  assert.equal(current.detached, false);
  // A narrowed range keeps its top, so the anchored page is filled from
  // below until it holds the first unread row, and a newer page until it
  // continues the tail: no row between is skipped.
  narrow = true;
  await ui.act(async () => {
    await current.reopen('60');
  });
  assert.deepEqual(sequences(), range(51, 100));
  assert.deepEqual(
    calls.slice(-2).map((a) => (a.action === 'history' ? a.before : null)),
    ['101', '76'],
  );
  await ui.act(async () => {
    await current.loadNewer();
  });
  assert.deepEqual(sequences(), range(51, 140));
  assert.deepEqual(
    calls.slice(-2).map((a) => (a.action === 'history' ? a.before : null)),
    ['151', '126'],
  );
  assert.equal(current.detached, false);
  // A published head the history cannot reach: the forward read that adds
  // no row attaches the window, so paging stops there.
  view.rerender(createElement(History, { revision: 5, position: '200' }));
  await ui.waitFor(() => assert.equal(current.busy, false));
  await ui.act(async () => {
    await current.reopen('60');
  });
  await ui.act(async () => {
    await current.loadNewer();
  });
  assert.equal(current.detached, true);
  await ui.act(async () => {
    await current.loadNewer();
  });
  assert.deepEqual(sequences(), range(51, 140));
  assert.equal(current.detached, false);
  // A narrowed range can hold only sequences above the head and come back
  // empty; the rows after the read pointer are then read directly.
  head = 105;
  view.rerender(createElement(History, { revision: 6, position: '105' }));
  await ui.waitFor(() => assert.equal(current.busy, false));
  await ui.act(async () => {
    await current.reopen('100');
  });
  assert.deepEqual(sequences(), range(101, 105));
  assert.equal(current.detached, false);
  assert.deepEqual(calls.slice(-2), [
    { action: 'history', channel: channelId, before: '141' },
    { action: 'history', channel: channelId, before: null, after: '100' },
  ]);
});
