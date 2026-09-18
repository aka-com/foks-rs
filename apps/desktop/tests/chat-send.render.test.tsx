import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, useState } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import { installDom } from './lib/dom-harness';
import type { Bridge } from '../src/bridge';
import type { Location } from '../src/location';

installDom({ body: '<div id="overlays"></div>', timers: true });
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
test.afterEach(() => ui.cleanup());
test.after(async () => {
  await vite.close();
});
function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
async function setup(override?: (base: Bridge) => Bridge) {
  const { ChatInboxProvider } = await vite.ssrLoadModule(
    '/src/chat/inbox-provider.tsx',
  );
  const { ChatTab } = await vite.ssrLoadModule('/src/screens/chat-tab.tsx');
  const { OverlayProvider } = await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  );
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const snapshot = {
    ...FIXTURE,
    servers: FIXTURE.servers.map((s) =>
      s.id === 'acme'
        ? {
            ...s,
            compatibility: { status: 'not-required' as const },
            capabilities: { chat: true },
          }
        : s,
    ),
  };
  const base = mockBridge(snapshot);
  const bridge = override?.(base) ?? base;
  function Host() {
    const [visible, setVisible] = useState(true);
    const [partial, setPartial] = useState(false);
    const shown = partial
      ? {
          ...snapshot,
          stores: snapshot.stores.filter((store) => store.id !== 'team:eng'),
          profileInventory: snapshot.profileInventory.map((profile) =>
            profile.profile === 'acme'
              ? { ...profile, teams: 'unavailable' as const }
              : profile,
          ),
        }
      : snapshot;
    const [location, setLocation] = useState<Location>({
      kind: 'chat',
      ref: 'team:eng',
      channel: 'ab'.repeat(16),
    });
    return createElement(OverlayProvider, {
      backgroundRef: { current: null },
      portalRoot: document.getElementById('overlays'),
      children: createElement(ChatInboxProvider, {
        bridge,
        snapshot: shown,
        children: createElement(
          'div',
          null,
          createElement(
            'button',
            { onClick: () => setVisible((v) => !v) },
            'Toggle conversation',
          ),
          createElement(
            'button',
            { onClick: () => setPartial((value) => !value) },
            'Toggle partial catalog',
          ),
          visible
            ? createElement(ChatTab, {
                bridge,
                snapshot: shown,
                location,
                onNavigate: setLocation,
              })
            : createElement('p', null, 'Other view'),
        ),
      }),
    });
  }
  ui.render(createElement(Host));
  await ui.screen.findByText('Team chat is ready.');
  await ui.waitFor(() =>
    assert.ok(
      ui.screen.getByRole<HTMLTextAreaElement>('textbox', { name: 'Message' }),
    ),
  );
}

test('optimistic submission survives conversation unmount during preparation and leaves the next draft editable', async () => {
  const gate = deferred();
  let preparations = 0;
  let attempts = 0;
  await setup((base) => ({
    ...base,
    chat: async (store, action, view) => {
      if (action.action === 'prepare-message') {
        preparations++;
        await gate.promise;
      }
      if (action.action === 'attempt') attempts++;
      return base.chat(store, action, view);
    },
  }));
  const field = ui.screen.getByRole<HTMLTextAreaElement>('textbox', {
    name: 'Message',
  });
  ui.fireEvent.change(field, { target: { value: 'first message' } });
  await ui.waitFor(() =>
    assert.equal(
      ui.screen.getByRole<HTMLButtonElement>('button', { name: 'Send' })
        .disabled,
      false,
    ),
  );
  ui.fireEvent.click(
    ui.screen.getByRole<HTMLButtonElement>('button', { name: 'Send' }),
  );
  await ui.screen.findByText('first message');
  assert.equal(field.disabled, false);
  assert.equal(field.value, '');
  assert.ok(document.querySelector('.chat-outgoing:not(.sent)'));
  ui.fireEvent.change(field, { target: { value: 'next draft' } });
  assert.equal(
    ui.screen.getByRole<HTMLButtonElement>('button', { name: 'Send' }).disabled,
    true,
  );
  ui.fireEvent.click(
    ui.screen.getByRole<HTMLButtonElement>('button', {
      name: 'Toggle conversation',
    }),
  );
  await ui.screen.findByText('Other view');
  gate.resolve();
  await ui.waitFor(() => assert.equal(attempts, 1));
  ui.fireEvent.click(
    ui.screen.getByRole<HTMLButtonElement>('button', {
      name: 'Toggle conversation',
    }),
  );
  await ui.screen.findByText('first message');
  await ui.waitFor(() =>
    assert.equal(
      ui.screen.getByRole<HTMLTextAreaElement>('textbox', { name: 'Message' })
        .value,
      'next draft',
    ),
  );
  assert.equal(preparations, 1);
  assert.equal(ui.screen.getAllByText('first message').length, 1);
});

test('partial catalog omission preserves both a submitted message and the next unsent draft', async () => {
  const gate = deferred();
  let attempts = 0;
  await setup((base) => ({
    ...base,
    chat: async (store, action, view) => {
      if (action.action === 'prepare-message') await gate.promise;
      if (action.action === 'attempt') attempts++;
      return base.chat(store, action, view);
    },
  }));
  try {
    const field = ui.screen.getByRole<HTMLTextAreaElement>('textbox', {
      name: 'Message',
    });
    ui.fireEvent.change(field, { target: { value: 'retained submission' } });
    await ui.waitFor(() =>
      assert.equal(
        ui.screen.getByRole<HTMLButtonElement>('button', { name: 'Send' })
          .disabled,
        false,
      ),
    );
    ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Send' }));
    await ui.screen.findByText('retained submission');
    ui.fireEvent.change(field, { target: { value: 'retained draft' } });
    ui.fireEvent.click(
      ui.screen.getByRole('button', { name: 'Toggle partial catalog' }),
    );
    assert.equal(ui.screen.queryByRole('textbox', { name: 'Message' }), null);
    ui.fireEvent.click(
      ui.screen.getByRole('button', { name: 'Toggle partial catalog' }),
    );
    gate.resolve();
    await ui.waitFor(() =>
      assert.equal(
        ui.screen.getByRole<HTMLTextAreaElement>('textbox', { name: 'Message' })
          .value,
        'retained draft',
      ),
    );
    await ui.waitFor(() => assert.equal(attempts, 1));
    assert.equal(ui.screen.getAllByText('retained submission').length, 1);
  } finally {
    gate.resolve();
  }
});

test('drafts survive ordinary navigation without requiring a send or persistence', async () => {
  await setup();
  ui.fireEvent.change(
    ui.screen.getByRole<HTMLTextAreaElement>('textbox', { name: 'Message' }),
    { target: { value: 'unsent draft' } },
  );
  ui.fireEvent.click(
    ui.screen.getByRole<HTMLButtonElement>('button', {
      name: 'Toggle conversation',
    }),
  );
  await ui.screen.findByText('Other view');
  ui.fireEvent.click(
    ui.screen.getByRole<HTMLButtonElement>('button', {
      name: 'Toggle conversation',
    }),
  );
  await ui.waitFor(() =>
    assert.equal(
      ui.screen.getByRole<HTMLTextAreaElement>('textbox', { name: 'Message' })
        .value,
      'unsent draft',
    ),
  );
});

test('optimistic rows follow the bottom without pulling a reader away from older messages', async () => {
  const gate = deferred();
  await setup((base) => ({
    ...base,
    chat: async (store, action, view) => {
      if (action.action === 'prepare-message') await gate.promise;
      return base.chat(store, action, view);
    },
  }));
  try {
    const scroller = document.querySelector<HTMLDivElement>('.chat-messages')!;
    Object.defineProperty(scroller, 'clientHeight', {
      configurable: true,
      value: 200,
    });
    Object.defineProperty(scroller, 'scrollHeight', {
      configurable: true,
      get: () => 500 + scroller.querySelectorAll('.chat-outgoing').length * 100,
    });
    scroller.scrollTop = 300;
    ui.fireEvent.scroll(scroller);
    ui.fireEvent.change(ui.screen.getByRole('textbox', { name: 'Message' }), {
      target: { value: 'visible immediately' },
    });
    await ui.waitFor(() =>
      assert.equal(
        ui.screen.getByRole<HTMLButtonElement>('button', { name: 'Send' })
          .disabled,
        false,
      ),
    );
    ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Send' }));
    await ui.screen.findByText('visible immediately');
    assert.equal(scroller.scrollTop, 600);
    scroller.scrollTop = 0;
    ui.fireEvent.scroll(scroller);
    gate.resolve();
    await ui.waitFor(() =>
      assert.ok(!document.querySelector('.chat-outgoing:not(.sent)')),
    );
    assert.equal(scroller.scrollTop, 0);
  } finally {
    gate.resolve();
  }
});

test('confirmation becomes Sent independently of a stalled history refresh', async () => {
  const gate = deferred();
  let sent = false;
  await setup((base) => ({
    ...base,
    chat: async (store, action, view) => {
      if (action.action === 'history' && sent) await gate.promise;
      const reply = await base.chat(store, action, view);
      if (action.action === 'attempt') sent = true;
      return reply;
    },
  }));
  try {
    ui.fireEvent.change(
      ui.screen.getByRole<HTMLTextAreaElement>('textbox', { name: 'Message' }),
      { target: { value: 'confirmed before refresh' } },
    );
    await ui.waitFor(() =>
      assert.equal(
        ui.screen.getByRole<HTMLButtonElement>('button', { name: 'Send' })
          .disabled,
        false,
      ),
    );
    ui.fireEvent.click(
      ui.screen.getByRole<HTMLButtonElement>('button', { name: 'Send' }),
    );
    await ui.screen.findByText('Sent');
    assert.ok(document.querySelector('.chat-outgoing.sent'));
    assert.equal(
      ui.screen.getByRole<HTMLTextAreaElement>('textbox', { name: 'Message' })
        .disabled,
      false,
    );
  } finally {
    gate.resolve();
  }
});
