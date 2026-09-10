import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, StrictMode } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import { installDom } from './lib/dom-harness';
import type { Bridge } from '../src/bridge';
installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div>',
  timers: true,
});
let ui: typeof import('@testing-library/react');
let vite: ViteDevServer;
test.before(async () => {
  ui = await import('@testing-library/react');
  vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
});
test.afterEach(() => ui.cleanup());
test.after(async () => {
  await vite.close();
});
async function setup(override?: (b: Bridge) => Bridge, waitForHistory = true) {
  const { ChatScreen } = await vite.ssrLoadModule(
    '/src/screens/chat-screen.tsx',
  );
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const bridge = mockBridge(FIXTURE);
  const rendered = ui.render(
    createElement(
      StrictMode,
      null,
      createElement(ChatScreen, {
        world: FIXTURE,
        bridge: override?.(bridge) ?? bridge,
        location: { kind: 'team-chat', ref: 'team:eng' },
        onNavigate: () => {},
      }),
    ),
  );
  if (waitForHistory) await ui.screen.findByText('Team chat is ready.');
  return rendered;
}
test('composer suppresses IME and repeated Enter, and renders hostile text safely', async () => {
  let preparations = 0;
  let attempts = 0;
  await setup((base) => ({
    ...base,
    chat: async (store, action) => {
      if (action.action === 'prepare-message') preparations++;
      if (action.action === 'attempt') attempts++;
      return base.chat(store, action);
    },
  }));
  const composer = ui.screen.getByRole('textbox', { name: 'Message' });
  ui.fireEvent.change(composer, {
    target: { value: '<img src=x onerror=alert(1)>' },
  });
  ui.fireEvent.keyDown(composer, {
    key: 'Enter',
    isComposing: true,
    keyCode: 229,
  });
  assert.equal(preparations, 0);
  ui.fireEvent.keyDown(composer, { key: 'Enter' });
  ui.fireEvent.keyDown(composer, { key: 'Enter' });
  await ui.screen.findByText('<img src=x onerror=alert(1)>');
  assert.equal(preparations, 1);
  assert.equal(attempts, 1);
  assert.equal(document.querySelector('.chat-messages img'), null);
  assert.equal((composer as HTMLTextAreaElement).value, '');
});
test('lost preparation reply reuses its submission and sends exactly once', async () => {
  const submissions: string[] = [];
  let attempts = 0;
  let dropped = false;
  await setup((base) => ({
    ...base,
    chat: async (store, action) => {
      const reply = await base.chat(store, action);
      if (action.action === 'prepare-message') {
        submissions.push(action.submission);
        if (!dropped) {
          dropped = true;
          throw {
            code: 'ambiguous',
            message: 'Preparation reply lost',
            ambiguous: true,
            retryable: false,
            fatal: false,
          };
        }
      }
      if (action.action === 'attempt') attempts++;
      return reply;
    },
  }));
  ui.fireEvent.change(ui.screen.getByRole('textbox', { name: 'Message' }), {
    target: { value: 'recover this message' },
  });
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Send' }));
  await ui.screen.findByText('Preparation reply lost');
  assert.equal(attempts, 0);
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Recover preparation' }),
  );
  await ui.screen.findByText('recover this message');
  assert.equal(new Set(submissions).size, 1);
  assert.equal(submissions.length, 2);
  assert.equal(attempts, 1);
});
test('closing a conversation during preparation never starts delivery', async () => {
  let release!: () => void;
  let prepared!: () => void;
  const reached = new Promise<void>((resolve) => {
    prepared = resolve;
  });
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  let attempts = 0;
  const rendered = await setup((base) => ({
    ...base,
    chat: async (store, action) => {
      const result = await base.chat(store, action);
      if (action.action === 'prepare-message') {
        prepared();
        await gate;
      }
      if (action.action === 'attempt') attempts++;
      return result;
    },
  }));
  ui.fireEvent.change(ui.screen.getByRole('textbox', { name: 'Message' }), {
    target: { value: 'leave prepared' },
  });
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Send' }));
  await reached;
  rendered.unmount();
  await ui.act(async () => {
    release();
    await gate;
  });
  assert.equal(attempts, 0);
  assert.equal(document.body.textContent?.includes('leave prepared'), false);
});

test('definite preparation errors allow input correction', async () => {
  await setup((base) => ({
    ...base,
    chat: async (store, action) => {
      if (action.action === 'prepare-channel')
        throw {
          code: 'chat-invalid-input',
          message: 'Use an empty name for general.',
          fatal: false,
          retryable: false,
          ambiguous: false,
        };
      return base.chat(store, action);
    },
  }));
  const name = ui.screen.getByRole('textbox', { name: 'Channel name' });
  ui.fireEvent.change(name, { target: { value: 'general' } });
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Create channel' }));
  await ui.screen.findByText('Use an empty name for general.');
  assert.equal((name as HTMLInputElement).disabled, false);
  ui.fireEvent.change(name, { target: { value: 'valid' } });
  assert.equal((name as HTMLInputElement).value, 'valid');
});

test('prepending older messages preserves scroll position', async () => {
  await setup((base) => ({
    ...base,
    chat: async (store, action) => {
      const reply = await base.chat(store, action);
      if (action.action === 'history' && reply.result.kind === 'history') {
        reply.result.messages =
          action.before === null
            ? [
                {
                  id: 'cd'.repeat(16),
                  sequence: '3',
                  sender: null,
                  content: { kind: 'text', text: 'Team chat is ready.' },
                },
              ]
            : [
                {
                  id: '01'.repeat(16),
                  sequence: '1',
                  sender: null,
                  content: { kind: 'text', text: 'First earlier message' },
                },
                {
                  id: '02'.repeat(16),
                  sequence: '2',
                  sender: null,
                  content: { kind: 'text', text: 'Second earlier message' },
                },
              ];
        reply.result.before = action.before === null ? '3' : null;
      }
      return reply;
    },
  }));
  const scroll = ui.screen.getByLabelText('Message history');
  Object.defineProperty(scroll, 'scrollHeight', {
    configurable: true,
    get: () => document.querySelectorAll('.chat-message').length * 100,
  });
  scroll.scrollTop = 20;
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Load older messages' }),
  );
  await ui.screen.findByText('First earlier message');
  assert.equal(scroll.scrollTop, 220);
  assert.deepEqual(
    [...document.querySelectorAll('.chat-message p')].map((p) => p.textContent),
    ['First earlier message', 'Second earlier message', 'Team chat is ready.'],
  );
});

test('refresh resets a disjoint window so older pagination reaches the gap', async () => {
  let latest = 100;
  const cursors: (string | null)[] = [];
  await setup((base) => ({
    ...base,
    chat: async (store, action) => {
      const reply = await base.chat(store, action);
      if (action.action === 'history' && reply.result.kind === 'history') {
        cursors.push(action.before);
        const end = action.before ? Number(action.before) - 1 : latest;
        reply.result.messages = Array.from({ length: 50 }, (_, i) => {
          const n = end - 49 + i;
          return {
            id: n.toString(16).padStart(32, '0'),
            sequence: String(n),
            sender: null,
            content: {
              kind: 'text' as const,
              text: n === 100 ? 'Team chat is ready.' : `Message ${n}`,
            },
          };
        });
        reply.result.before = String(end - 49);
      }
      return reply;
    },
  }));
  latest = 200;
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Refresh messages' }),
  );
  await ui.screen.findByText('Message 200');
  assert.equal(ui.screen.queryByText('Team chat is ready.'), null);
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Load older messages' }),
  );
  await ui.screen.findByText('Message 101');
  assert.equal(cursors.at(-1), '151');
});

test('verification warnings survive unrelated pages until retained rows are rechecked', async () => {
  let stage = 0;
  await setup((base) => ({
    ...base,
    chat: async (store, action) => {
      const reply = await base.chat(store, action);
      if (action.action === 'history' && reply.result.kind === 'history') {
        const original = {
          id: '03'.repeat(16),
          sequence: '3',
          sender: null,
          content: { kind: 'text' as const, text: 'Team chat is ready.' },
        };
        const next = {
          id: '04'.repeat(16),
          sequence: '4',
          sender: null,
          content: { kind: 'text' as const, text: 'Next message' },
        };
        reply.result.messages =
          stage === 0 ? [original] : stage === 1 ? [next] : [original, next];
        reply.result.before = stage === 1 ? '4' : '3';
        reply.result.missing_predecessors = stage === 0 ? ['1'] : [];
      }
      return reply;
    },
  }));
  assert.ok(ui.screen.getByText(/incomplete verification/));
  stage = 1;
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Refresh messages' }),
  );
  await ui.screen.findByText('Next message');
  assert.ok(ui.screen.getByText(/incomplete verification/));
  stage = 2;
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: 'Refresh messages' }),
  );
  await ui.waitFor(() =>
    assert.ok(ui.screen.queryByText(/incomplete verification/) === null),
  );
});

for (const offline of [true, false]) {
  test(`pending preparations can be cancelled when channels are ${offline ? 'offline' : 'restricted'}`, async () => {
    let seeded = false;
    await setup(
      (base) => ({
        ...base,
        chat: async (store, action) => {
          if (!seeded) {
            seeded = true;
            await base.chat(store, {
              action: 'prepare-message',
              channel: 'ab'.repeat(16),
              submission: '77'.repeat(16),
              text: 'Cancel this preparation',
            });
          }
          if (offline && action.action === 'channels')
            throw {
              code: 'offline',
              message: 'Offline',
              fatal: false,
              retryable: true,
              ambiguous: false,
            };
          const reply = await base.chat(store, action);
          if (!offline && reply.result.kind === 'channels')
            reply.result.channels = reply.result.channels.map((channel) => ({
              ...channel,
              readable: false,
            }));
          return reply;
        },
      }),
      false,
    );
    await ui.screen.findByText(
      offline ? 'Offline' : 'Your current role cannot read this channel.',
    );
    ui.fireEvent.click(
      ui.screen.getByRole('button', { name: 'Cancel preparation' }),
    );
    await ui.screen.findByText(/Message · cancelled/);
    ui.fireEvent.click(
      ui.screen.getByRole('button', { name: 'Finish cleanup' }),
    );
    await ui.waitFor(() =>
      assert.ok(ui.screen.queryByText(/Message · cancelled/) === null),
    );
  });
}

test('durable pending refresh replaces stale prepared state after lost delivery and status replies', async () => {
  let uncertain = false;
  await setup((base) => ({
    ...base,
    chat: async (store, action) => {
      if (action.action === 'attempt') {
        uncertain = true;
        throw {
          code: 'ambiguous',
          message: 'Delivery reply lost',
          fatal: false,
          ambiguous: true,
          retryable: false,
        };
      }
      if (action.action === 'status')
        throw {
          code: 'offline',
          message: 'Status unavailable',
          fatal: false,
          ambiguous: false,
          retryable: true,
        };
      const reply = await base.chat(store, action);
      if (uncertain && reply.result.kind === 'pending')
        reply.result.operations = reply.result.operations.map((op) => ({
          ...op,
          state: 'uncertain',
        }));
      return reply;
    },
  }));
  ui.fireEvent.change(ui.screen.getByRole('textbox', { name: 'Message' }), {
    target: { value: 'recover delivery' },
  });
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Send' }));
  await ui.screen.findByRole('button', { name: 'Check delivery' });
  assert.equal(
    ui.screen.queryByRole('button', { name: 'Cancel preparation' }),
    null,
  );
  assert.equal(
    ui.screen.queryByRole('button', { name: 'Send prepared' }),
    null,
  );
});
