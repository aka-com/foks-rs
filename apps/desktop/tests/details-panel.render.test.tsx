import assert from 'node:assert/strict';
import test from 'node:test';
import { PROTOCOL_CAPABILITIES } from '../src/model/types';
import { createElement, StrictMode } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import type { Bridge, ReadItemResponse } from '../src/bridge';
import type { GuardVerdict, LocationStore } from '../src/location';
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
  selectItem?: (
    item: DetailsPanelProps['snapshot']['items'][number],
  ) => boolean,
  /** Present when the panel is mounted under a store's guard registry. */
  store?: LocationStore,
) {
  const { DetailsPanel } = (await vite.ssrLoadModule(
    '/src/screens/details-panel.tsx',
  )) as typeof import('../src/screens/details-panel');
  const { NavigationGuardProvider } = (await vite.ssrLoadModule(
    '/src/navigation-guard.tsx',
  )) as typeof import('../src/navigation-guard');
  const { ToastProvider, ToastController } = (await vite.ssrLoadModule(
    '/kit/toasts.tsx',
  )) as typeof import('../kit/toasts');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const subject = FIXTURE.items.find(
    selectItem ?? ((item) => item.path === '/env/prod/DATABASE_URL'),
  )!;
  assert.ok(subject);
  const bridge = mockBridge();
  const props: DetailsPanelProps = {
    snapshot: FIXTURE,
    bridge: readItem ? { ...bridge, readItem } : bridge,
    selection: { store: subject.store, path: subject.path },
    onClose: () => {},
    onDelete: () => {},
    onConflict: () => {},
    onApplied: async () => {},
    onCommandError: () => {},
    onMutationError: async () => {},
  };
  const panel = () =>
    createElement(ToastProvider, {
      controller: new ToastController(),
      children: createElement(DetailsPanel, props),
    });
  const draw = () =>
    createElement(
      StrictMode,
      null,
      store
        ? createElement(NavigationGuardProvider, { store, children: panel() })
        : panel(),
    );
  return { props, draw, subject, bridge };
}

test('a pending read cannot reveal into a different selection', async () => {
  let finish!: (response: ReadItemResponse) => void;
  const pending = new Promise<ReadItemResponse>((resolve) => {
    finish = resolve;
  });
  const p = await setup(() => pending);
  const r = ui.render(p.draw());
  ui.fireEvent.click(r.getByRole('button', { name: 'Show' }));
  const other = p.props.snapshot.items.find((item) => item.kind === 'Secret')!;
  p.props.selection = { store: other.store, path: other.path };
  r.rerender(p.draw());
  await ui.act(async () => {
    finish({
      store: p.subject.store,
      path: p.subject.path,
      version: p.subject.version,
      value: '/late-target',
    });
    await pending;
  });
  assert.ok(r.queryByText('/late-target', { exact: false }) === null);
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
  ui.fireEvent.click(r.getByRole('button', { name: 'Show' }));
  await ui.waitFor(() => assert.equal(reads, 1));

  const store = p.props.snapshot.stores.find(
    (entry) => entry.id === p.subject.store,
  );
  assert.ok(store);
  p.props.snapshot = {
    ...p.props.snapshot,
    servers: p.props.snapshot.servers.map((server) =>
      server.id === store.server
        ? {
            ...server,
            compatibility: {
              status: 'required' as const,
              capabilities: PROTOCOL_CAPABILITIES,
              expiresAt: 1,
            },
          }
        : server,
    ),
    observedExpiredLeases: [{ profile: store.server, expiresAt: 1 }],
  };
  p.props.accessGeneration = 1;
  r.rerender(p.draw());
  await ui.act(async () => {
    pending[0]?.({
      store: p.subject.store,
      path: p.subject.path,
      version: p.subject.version,
      value: '/expired-flight',
    });
  });
  assert.equal(r.queryByText('/expired-flight', { exact: false }), null);

  p.props.snapshot = {
    ...p.props.snapshot,
    servers: p.props.snapshot.servers.map((server) =>
      server.id === store.server
        ? {
            ...server,
            compatibility: {
              status: 'required' as const,
              capabilities: PROTOCOL_CAPABILITIES,
              expiresAt: Math.floor(Date.now() / 1000) + 3_600,
            },
          }
        : server,
    ),
    observedExpiredLeases: [],
  };
  p.props.accessGeneration = 2;
  r.rerender(p.draw());
  await ui.waitFor(() =>
    ui.fireEvent.click(r.getByRole('button', { name: 'Show' })),
  );
  await ui.waitFor(() => assert.equal(reads, 2));
  await ui.act(async () => {
    pending[1]?.({
      store: p.subject.store,
      path: p.subject.path,
      version: p.subject.version,
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
  ui.fireEvent.click(first.getByRole('button', { name: 'Show' }));
  await ui.waitFor(() => assert.equal(reads, 1));
  first.unmount();
  const second = ui.render(p.draw());
  ui.fireEvent.click(second.getByRole('button', { name: 'Show' }));
  await ui.waitFor(() => assert.equal(reads, 2));
  await ui.act(async () => {
    pending[0]?.({
      store: p.subject.store,
      path: p.subject.path,
      version: p.subject.version,
      value: '/old-session',
    });
    pending[1]?.({
      store: p.subject.store,
      path: p.subject.path,
      version: p.subject.version,
      value: '/new-session',
    });
  });
  assert.equal(second.queryByText('/old-session', { exact: false }), null);
  await ui.waitFor(() =>
    assert.ok(second.getByText('/new-session', { exact: false })),
  );
});

test('a mismatched version is rejected and Show retries the read', async () => {
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
  ui.fireEvent.click(r.getByRole('button', { name: 'Show' }));
  await ui.waitFor(() => assert.ok(r.getByRole('alert')));
  assert.ok(r.queryByText('/wrong-target', { exact: false }) === null);
  const before = reads;
  ui.fireEvent.click(r.getByRole('button', { name: 'Show' }));
  await ui.waitFor(() => assert.ok(reads > before));
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
  ui.fireEvent.change(username, { target: { value: 'satoshi-next' } });
  ui.fireEvent.change(password, { target: { value: 'correct horse' } });
  ui.fireEvent.click(r.getByRole('button', { name: 'Save changes' }));

  await ui.waitFor(() => assert.match(saved, /username: satoshi-next/));
  assert.match(saved, /password: correct horse/);
  assert.match(saved, /url: https:\/\/github\.com\/login/);
});

/* ------------------------------------------------- the unsaved-edit guard -- */

/** The shell's prompter, reduced to what these tests ask of it. */
function prompts() {
  type Asked = Extract<GuardVerdict, { verdict: 'prompt' }>;
  const asked: Asked[] = [];
  const answers: ((confirmed: boolean) => void)[] = [];
  return {
    asked,
    answer: async (index: number, confirmed: boolean) => {
      await ui.act(async () => {
        answers[index]?.(confirmed);
        await Promise.resolve();
      });
    },
    prompter: (verdict: Asked) => {
      asked.push(verdict);
      return new Promise<boolean>((resolve) => answers.push(resolve));
    },
  };
}

/** The panel with the login's editor open, under `store`'s guards. */
async function editingLogin(store: LocationStore) {
  const p = await setup(
    undefined,
    (item) => item.path === '/logins/github.com',
    store,
  );
  const r = ui.render(p.draw());
  ui.fireEvent.click(r.getByRole('button', { name: 'Edit' }));
  const username = await r.findByLabelText('User name');
  return { p, r, username };
}

/** Runs the navigation the guard is asked about. */
async function leave(store: LocationStore): Promise<void> {
  await ui.act(async () => {
    store.navigate({ kind: 'files' });
    await Promise.resolve();
  });
}

async function guardStore(): Promise<LocationStore> {
  const { LocationStore } = (await vite.ssrLoadModule(
    '/src/location.ts',
  )) as typeof import('../src/location');
  return new LocationStore();
}

test('an unchanged editor closes without a discard prompt', async () => {
  const store = await guardStore();
  const { asked, prompter } = prompts();
  store.setPrompter(prompter);
  const { username } = await editingLogin(store);
  assert.ok(username);

  await leave(store);
  assert.deepEqual(asked, []);
  assert.deepEqual(store.getSnapshot().location, { kind: 'files' });
});

test('unsaved edits ask before a navigation, naming the item', async () => {
  const store = await guardStore();
  const { asked, prompter } = prompts();
  store.setPrompter(prompter);
  const { username } = await editingLogin(store);
  ui.fireEvent.change(username, { target: { value: 'satoshi-next' } });

  await leave(store);
  assert.equal(asked.length, 1);
  assert.equal(asked[0]?.title, 'Discard changes?');
  assert.equal(asked[0]?.body, 'Your edits to github.com have not been saved.');
  assert.equal(asked[0]?.confirm, 'Discard');
  // Nothing moves while the question is open.
  assert.deepEqual(store.getSnapshot().location, { kind: 'all' });
});

test('confirming the discard closes the editor and makes the move', async () => {
  const store = await guardStore();
  const { answer, prompter } = prompts();
  store.setPrompter(prompter);
  const { r, username } = await editingLogin(store);
  ui.fireEvent.change(username, { target: { value: 'satoshi-next' } });

  await leave(store);
  await answer(0, true);
  assert.deepEqual(store.getSnapshot().location, { kind: 'files' });
  // The panel the navigation lands on is clean: the editor is gone.
  await ui.waitFor(() => assert.ok(r.getByRole('button', { name: 'Edit' })));
  assert.equal(r.queryByLabelText('User name'), null);
});

test('cancelling the discard keeps the draft and the reader in place', async () => {
  const store = await guardStore();
  const { answer, prompter } = prompts();
  store.setPrompter(prompter);
  const { r, username } = await editingLogin(store);
  ui.fireEvent.change(username, { target: { value: 'satoshi-next' } });

  await leave(store);
  await answer(0, false);
  assert.deepEqual(store.getSnapshot().location, { kind: 'all' });
  assert.equal(
    (r.getByLabelText('User name') as HTMLInputElement).value,
    'satoshi-next',
  );
});

test('reselecting the item being edited asks nothing; another item asks', async () => {
  const store = await guardStore();
  const { asked, prompter } = prompts();
  store.setPrompter(prompter);
  const { p, username } = await editingLogin(store);
  ui.fireEvent.change(username, { target: { value: 'satoshi-next' } });
  const edited = p.props.selection;
  assert.ok(edited);
  const other = p.props.snapshot.items.find(
    (item) => item.store !== edited.store || item.path !== edited.path,
  );
  assert.ok(other);

  await ui.act(async () => {
    store.select(edited);
    await Promise.resolve();
  });
  assert.deepEqual(asked, []);
  assert.deepEqual(store.getSnapshot().selection, edited);

  await ui.act(async () => {
    store.select({ store: other.store, path: other.path });
    await Promise.resolve();
  });
  assert.equal(asked.length, 1);
  assert.deepEqual(store.getSnapshot().selection, edited);
});
