import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, StrictMode } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import type { Bridge, ReadItemResponse } from '../src/bridge';
import type { Selection } from '../src/location';
import type { DetailsPanelProps } from '../src/screens/details-panel';
import { installDom } from './lib/dom-harness';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
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
test.after(async () => vite.close());

async function setup(
  readItem?: Bridge['readItem'],
  selectItem?: (item: DetailsPanelProps['world']['items'][number]) => boolean,
) {
  const { DetailsPanel } = (await vite.ssrLoadModule(
    '/src/screens/details-panel.tsx',
  )) as typeof import('../src/screens/details-panel');
  const { ToastProvider, ToastController } = (await vite.ssrLoadModule(
    '/kit/toasts.tsx',
  )) as typeof import('../kit/toasts');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const link = FIXTURE.items.find(
    selectItem ?? ((item) => item.kind === 'Link'),
  )!;
  assert.ok(link);
  const bridge = mockBridge();
  const selected: Selection[] = [];
  const props: DetailsPanelProps = {
    world: FIXTURE,
    bridge: readItem ? { ...bridge, readItem } : bridge,
    selection: { store: link.store, path: link.path },
    onSelect: (selection) => {
      selected.push(selection);
    },
    onClose: () => {},
    onDelete: () => {},
    onConflict: () => {},
    onApplied: async () => {},
    onCommandError: () => {},
    onMutationError: async () => {},
  };
  const draw = () =>
    createElement(
      StrictMode,
      null,
      createElement(ToastProvider, {
        controller: new ToastController(),
        children: createElement(DetailsPanel, props),
      }),
    );
  return { props, draw, selected, link, bridge };
}

test('link target loads at the selected version and opens in one click, including after blur', async () => {
  const p = await setup();
  const expected = await p.bridge.readItem({
    storeId: p.link.store,
    path: p.link.path,
    version: p.link.version,
  });
  const r = ui.render(p.draw());
  await ui.waitFor(() =>
    assert.ok(r.getByText(expected.value, { exact: false })),
  );
  ui.fireEvent(window, new Event('blur'));
  assert.ok(r.getByText(expected.value, { exact: false }));
  ui.fireEvent.click(r.getByRole('button', { name: 'Open target' }));
  assert.deepEqual(p.selected, [{ store: p.link.store, path: expected.value }]);
});

test('a pending link read cannot reveal into a different selection', async () => {
  let finish!: (response: ReadItemResponse) => void;
  const pending = new Promise<ReadItemResponse>((resolve) => {
    finish = resolve;
  });
  const p = await setup(() => pending);
  const r = ui.render(p.draw());
  const other = p.props.world.items.find((item) => item.kind === 'Secret')!;
  p.props.selection = { store: other.store, path: other.path };
  r.rerender(p.draw());
  await ui.act(async () => {
    finish({
      store: p.link.store,
      path: p.link.path,
      version: p.link.version,
      value: '/late-target',
    });
    await pending;
  });
  assert.ok(r.queryByText('/late-target', { exact: false }) === null);
  assert.deepEqual(p.selected, []);
});

test('an access generation quarantines old read flights across expiry and renewal', async () => {
  const pending: ((response: ReadItemResponse) => void)[] = [];
  let reads = 0;
  const p = await setup(() => {
    reads++;
    return new Promise<ReadItemResponse>((resolve) => {
      pending.push(resolve);
    });
  });
  const r = ui.render(p.draw());
  await ui.waitFor(() => assert.equal(reads, 1));

  const store = p.props.world.stores.find((entry) => entry.id === p.link.store);
  assert.ok(store);
  p.props.world = {
    ...p.props.world,
    servers: p.props.world.servers.map((server) =>
      server.id === store.server
        ? {
            ...server,
            compatibility: { status: 'required' as const, expiresAt: 1 },
          }
        : server,
    ),
    observedExpiredLeases: [{ profile: store.server, expiresAt: 1 }],
  };
  p.props.accessGeneration = 1;
  r.rerender(p.draw());
  await ui.act(async () => {
    pending[0]?.({
      store: p.link.store,
      path: p.link.path,
      version: p.link.version,
      value: '/expired-flight',
    });
  });
  assert.equal(r.queryByText('/expired-flight', { exact: false }), null);

  p.props.world = {
    ...p.props.world,
    servers: p.props.world.servers.map((server) =>
      server.id === store.server
        ? {
            ...server,
            compatibility: {
              status: 'required' as const,
              expiresAt: Math.floor(Date.now() / 1000) + 3_600,
            },
          }
        : server,
    ),
    observedExpiredLeases: [],
  };
  p.props.accessGeneration = 2;
  r.rerender(p.draw());
  await ui.waitFor(() => assert.equal(reads, 2));
  await ui.act(async () => {
    pending[1]?.({
      store: p.link.store,
      path: p.link.path,
      version: p.link.version,
      value: '/renewed-flight',
    });
  });
  await ui.waitFor(() =>
    assert.ok(r.getByText('/renewed-flight', { exact: false })),
  );
});

test('a remounted unlocked session cannot join an older pending read', async () => {
  const pending: ((response: ReadItemResponse) => void)[] = [];
  let reads = 0;
  const p = await setup(() => {
    reads++;
    return new Promise<ReadItemResponse>((resolve) => pending.push(resolve));
  });
  const first = ui.render(p.draw());
  await ui.waitFor(() => assert.equal(reads, 1));
  first.unmount();
  const second = ui.render(p.draw());
  await ui.waitFor(() => assert.equal(reads, 2));
  await ui.act(async () => {
    pending[0]?.({
      store: p.link.store,
      path: p.link.path,
      version: p.link.version,
      value: '/old-session',
    });
    pending[1]?.({
      store: p.link.store,
      path: p.link.path,
      version: p.link.version,
      value: '/new-session',
    });
  });
  assert.equal(second.queryByText('/old-session', { exact: false }), null);
  await ui.waitFor(() =>
    assert.ok(second.getByText('/new-session', { exact: false })),
  );
});

test('a mismatched link version is rejected and Open target retries the read', async () => {
  let reads = 0;
  const p = await setup(async (request) => {
    reads++;
    return {
      store: request.storeId,
      path: request.path,
      version: request.version + 1,
      value: '/wrong-target',
    };
  });
  const r = ui.render(p.draw());
  await ui.waitFor(() => assert.ok(r.getByRole('alert')));
  assert.ok(r.queryByText('/wrong-target', { exact: false }) === null);
  const before = reads;
  ui.fireEvent.click(r.getByRole('button', { name: 'Open target' }));
  await ui.waitFor(() => assert.ok(reads > before));
  assert.deepEqual(p.selected, []);
});

test('a login edits in structured fields and serializes through the existing draft value', async () => {
  const p = await setup(
    undefined,
    (item) => item.path === '/logins/github.com',
  );
  let saved = '';
  p.bridge.editTextItem = async (request) => {
    saved = request.value;
    return { applied: true };
  };
  const r = ui.render(p.draw());

  assert.ok(r.getByText('••••••••••••'));
  ui.fireEvent.click(r.getByRole('button', { name: 'Edit' }));
  const password = await r.findByLabelText('Password');
  const username = r.getByLabelText('User name');
  assert.equal(password.getAttribute('type'), 'password');
  assert.equal(r.queryByRole('textbox', { name: 'Contents' }), null);

  ui.fireEvent.click(r.getByRole('button', { name: 'Show' }));
  assert.equal(password.getAttribute('type'), 'text');
  ui.fireEvent.change(username, { target: { value: 'rae-next' } });
  ui.fireEvent.change(password, { target: { value: 'correct horse' } });
  ui.fireEvent.click(r.getByRole('button', { name: 'Save changes' }));

  await ui.waitFor(() => assert.match(saved, /username: rae-next/));
  assert.match(saved, /password: correct horse/);
  assert.match(saved, /url: https:\/\/github\.com\/login/);
});
