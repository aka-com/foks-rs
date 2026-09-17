import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import { installDom } from './lib/dom-harness';
import type { ChatAction, ChatReply } from '../src/chat-contract';
import type { ChatIntentPersistence } from '../src/chat/intent';
import type { useChatComposer as Hook } from '../src/chat/use-chat-composer';

installDom({ act: true });
let ui: typeof import('@testing-library/react');
let vite: ViteDevServer;
let useChatComposer: typeof Hook;
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
const channel = {
  id: 'ab'.repeat(16),
  name: 'general',
  description: null,
  admin: false,
  readable: true,
  writable: true,
  read_role: 'Member (0)',
  write_role: 'Member (0)',
};
const scope = {
  host: '02' + 'ab'.repeat(32),
  actor: '01' + 'ab'.repeat(32),
  store: {
    profile: 'local',
    account_alias: 'owner',
    team_alias: 'team',
    team_id: '03' + 'ab'.repeat(32),
  },
};
function reply(id: string, state: 'prepared' | 'confirmed'): ChatReply {
  return {
    scope,
    result: {
      kind: 'operation',
      operation: {
        id,
        channel: channel.id,
        kind: 'send-message',
        state,
        receipt:
          state === 'confirmed'
            ? { kind: 'message-sent', sequence: '2' }
            : null,
        rejection_code: null,
      },
    },
  };
}
async function setup(
  options: {
    refreshPending?: () => Promise<void>;
    load?: () => Promise<void>;
    request?: (action: ChatAction) => Promise<ChatReply>;
  } = {},
) {
  let current!: ReturnType<typeof Hook>;
  const calls: ChatAction[] = [];
  const persistence: ChatIntentPersistence = {
    key: 'scope/channel',
    load: async () => undefined,
    save: async () => {},
    clear: async () => {},
  };
  const request = async (action: ChatAction) => {
    calls.push(action);
    if (options.request) return options.request(action);
    if (action.action === 'prepare-message')
      return reply(action.submission, 'prepared');
    if (action.action === 'attempt' || action.action === 'status')
      return reply(action.operation, 'confirmed');
    throw new Error('Unexpected test request');
  };
  function Composer() {
    current = useChatComposer(
      channel,
      request,
      options.refreshPending ?? (async () => {}),
      options.load ?? (async () => {}),
      undefined,
      'store',
      persistence,
    );
    return createElement('span', null, current.draft);
  }
  ui.render(createElement(Composer));
  await ui.waitFor(() => assert.equal(current.intentLoading, false));
  return {
    get current() {
      return current;
    },
    calls,
    async send(text: string) {
      await ui.act(async () => {
        current.setDraft(text);
      });
      let pending!: Promise<void>;
      await ui.act(async () => {
        pending = current.send();
        await Promise.resolve();
      });
      return { pending };
    },
  };
}

test('confirmed delivery releases the composer before delayed history completes', async () => {
  const history = deferred();
  const h = await setup({ load: () => history.promise });
  const first = await h.send('first');
  try {
    assert.equal(h.calls.filter((a) => a.action === 'attempt').length, 1);
    assert.equal(h.current.sending, false);
    const second = await h.send('second');
    await second.pending;
    assert.equal(h.calls.filter((a) => a.action === 'attempt').length, 2);
    const ids = h.calls
      .filter((a) => a.action === 'prepare-message')
      .map((a) => a.submission);
    assert.equal(new Set(ids).size, 2);
  } finally {
    await ui.act(async () => {
      history.resolve();
      await first.pending;
    });
  }
});

test('pending-view refresh is not a prerequisite to attempting prepared delivery', async () => {
  const pendingView = deferred();
  const h = await setup({ refreshPending: () => pendingView.promise });
  const send = await h.send('send before optional refresh');
  try {
    assert.equal(h.calls.filter((a) => a.action === 'attempt').length, 1);
    assert.equal(h.current.sending, false);
  } finally {
    await ui.act(async () => {
      pendingView.resolve();
      await send.pending;
    });
  }
});

test('display refresh failure is reported separately and never retries delivery', async () => {
  const h = await setup({
    load: () => {
      throw {
        code: 'deadline-exceeded',
        message: 'History unavailable',
        retryable: true,
        fatal: false,
        ambiguous: false,
      };
    },
  });
  const send = await h.send('confirmed once');
  await send.pending;
  await ui.waitFor(() =>
    assert.equal(h.current.refreshError, 'History unavailable'),
  );
  assert.equal(h.current.sendError, '');
  assert.equal(h.current.sending, false);
  assert.equal(h.calls.filter((a) => a.action === 'attempt').length, 1);
});

test('late refresh failures cannot overwrite a later send state', async () => {
  const old = deferred();
  let loads = 0;
  const h = await setup({
    load: () => (++loads === 1 ? old.promise : Promise.resolve()),
  });
  const first = await h.send('first');
  await first.pending;
  const second = await h.send('second');
  await second.pending;
  await ui.act(async () => {
    old.reject(new Error('old refresh failed'));
    await Promise.resolve();
  });
  assert.equal(h.current.sendError, '');
  assert.equal(h.current.refreshError, '');
  assert.equal(h.current.sending, false);
});

test('ambiguous delivery can finish the composer while a read-only status check waits', async () => {
  const status = deferred();
  let pendingRefreshes = 0;
  const h = await setup({
    refreshPending: async () => {
      pendingRefreshes++;
    },
    request: async (action) => {
      if (action.action === 'prepare-message')
        return reply(action.submission, 'prepared');
      if (action.action === 'attempt')
        throw {
          code: 'ambiguous',
          message: 'Delivery unknown',
          ambiguous: true,
          retryable: false,
          fatal: false,
        };
      if (action.action === 'status') {
        await status.promise;
        return reply(action.operation, 'prepared');
      }
      throw new Error('Unexpected test request');
    },
  });
  const send = await h.send('may have been delivered');
  try {
    assert.equal(h.current.sending, false);
    assert.equal(h.current.sendError, 'Delivery unknown');
    assert.equal(pendingRefreshes, 1);
    assert.equal(h.calls.filter((a) => a.action === 'attempt').length, 1);
  } finally {
    await ui.act(async () => {
      status.resolve();
      await send.pending;
    });
  }
});
