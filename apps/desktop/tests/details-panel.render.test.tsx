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
  const { OverlayProvider } = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives');
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
    createElement(OverlayProvider, {
      backgroundRef: { current: document.getElementById('root') },
      portalRoot: document.getElementById('overlays')!,
      children: createElement(ToastProvider, {
        controller: new ToastController(),
        children: createElement(DetailsPanel, props),
      }),
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

for (const kind of ['Secret', 'File'] as const) {
  test(`${kind} actions remain visible but disabled after vault access is lost`, async () => {
    const p = await setup(undefined, (item) => item.kind === kind);
    const rendered = ui.render(p.draw());
    p.props.snapshot = {
      ...p.props.snapshot,
      storeInventory: p.props.snapshot.storeInventory.map((entry) =>
        entry.store === p.subject.store
          ? { ...entry, status: 'unavailable' as const }
          : entry,
      ),
    };
    rendered.rerender(p.draw());
    const close = rendered.getByRole('button', { name: 'Close' });
    const actions = rendered
      .getAllByRole('button')
      .filter((button) => button !== close) as HTMLButtonElement[];
    assert.ok(actions.length > 0);
    for (const button of actions) {
      assert.equal(button.disabled, true);
      assert.match(button.title, /unavailable|could not be loaded/i);
      const description = button.getAttribute('aria-describedby');
      assert.ok(
        description && document.getElementById(description)?.textContent,
      );
    }
    assert.equal((close as HTMLButtonElement).disabled, false);
  });
}

test('an open edit disables Save after access loss without discarding the draft or removing controls', async () => {
  const p = await setup(
    undefined,
    (item) => item.path === '/logins/github.com',
  );
  let writes = 0;
  p.bridge.editTextItem = async () => {
    writes++;
    return { applied: true };
  };
  const original = p.props.snapshot;
  const rendered = ui.render(p.draw());
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Edit' }));
  const username = await rendered.findByLabelText('User name');
  ui.fireEvent.change(username, { target: { value: 'retained-draft' } });
  p.props.snapshot = {
    ...original,
    storeInventory: original.storeInventory.map((entry) =>
      entry.store === p.subject.store
        ? { ...entry, status: 'unavailable' as const }
        : entry,
    ),
  };
  rendered.rerender(p.draw());
  const save = rendered.getByRole('button', {
    name: 'Save changes',
  }) as HTMLButtonElement;
  assert.equal(save.disabled, true);
  assert.match(save.title, /unavailable|could not be loaded/i);
  assert.equal(
    (rendered.getByRole('button', { name: 'Cancel' }) as HTMLButtonElement)
      .disabled,
    false,
  );
  assert.equal(
    (rendered.getByLabelText('User name') as HTMLInputElement).value,
    'retained-draft',
  );
  ui.fireEvent.click(save);
  assert.equal(writes, 0);
  p.props.snapshot = original;
  rendered.rerender(p.draw());
  assert.equal(
    (
      rendered.getByRole('button', {
        name: 'Save changes',
      }) as HTMLButtonElement
    ).disabled,
    false,
  );
});

test('an open team edit disables Save when its write authorization is revoked', async () => {
  const p = await setup(
    undefined,
    (item) => item.path === '/deploy/staging-token',
  );
  const rendered = ui.render(p.draw());
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Edit' }));
  await rendered.findByRole('button', { name: 'Save changes' });
  p.props.snapshot = {
    ...p.props.snapshot,
    parties: p.props.snapshot.parties.filter(
      (party) => party.store !== p.subject.store || party.label !== 'you',
    ),
  };
  rendered.rerender(p.draw());
  const save = rendered.getByRole('button', {
    name: 'Save changes',
  }) as HTMLButtonElement;
  assert.equal(save.disabled, true);
  assert.match(save.title, /permission to change/);
  assert.equal(
    (rendered.getByRole('button', { name: 'Cancel' }) as HTMLButtonElement)
      .disabled,
    false,
  );
});

test('access expiring between render and click disables Show and explains the refusal', async () => {
  let reads = 0;
  const p = await setup(async () => {
    reads++;
    throw new Error('must not read');
  });
  let now = 1;
  p.props.accessNow = () => now;
  const store = p.props.snapshot.stores.find(
    (entry) => entry.id === p.subject.store,
  )!;
  p.props.snapshot = {
    ...p.props.snapshot,
    observedExpiredLeases: [],
    servers: p.props.snapshot.servers.map((server) =>
      server.profileName === store.server
        ? {
            ...server,
            compatibility: {
              status: 'required' as const,
              expiresAt: 2,
              capabilities: PROTOCOL_CAPABILITIES,
            },
          }
        : server,
    ),
  };
  const rendered = ui.render(p.draw());
  now = 3;
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Show' }));
  await ui.waitFor(() =>
    assert.equal(
      (rendered.getByRole('button', { name: 'Show' }) as HTMLButtonElement)
        .disabled,
      true,
    ),
  );
  assert.match(rendered.container.textContent ?? '', /expired/);
  assert.equal(reads, 0);
});

test('an already-open Delete sheet disables its action when access is lost and re-enables after recovery', async () => {
  const p = await setup(
    undefined,
    (item) => item.path === '/logins/github.com',
  );
  const { WriteOverlay } = (await vite.ssrLoadModule(
    '/src/screens/write-workflows.tsx',
  )) as typeof import('../src/screens/write-workflows');
  const { OverlayProvider } = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives');
  const { ToastProvider, ToastController } = (await vite.ssrLoadModule(
    '/kit/toasts.tsx',
  )) as typeof import('../kit/toasts');
  let snapshot = p.props.snapshot;
  let writes = 0;
  p.bridge.removeItem = async () => {
    writes++;
    return { applied: true };
  };
  const controller = new ToastController();
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  const draw = () =>
    createElement(OverlayProvider, {
      backgroundRef: { current: null },
      portalRoot,
      children: createElement(ToastProvider, {
        controller,
        children: createElement(WriteOverlay, {
          snapshot,
          bridge: p.bridge,
          workflow: { kind: 'delete', item: p.subject },
          setWorkflow: () => {},
          onApplied: async () => {},
          onError: () => {},
          onMutationError: async () => {},
          onRefreshConflict: async () => null,
          onDiscardConflict: () => {},
          onDeleteConflict: async () => {},
          onOpenExisting: async () => {},
        }),
      }),
    });
  const rendered = ui.render(draw());
  snapshot = {
    ...snapshot,
    storeInventory: snapshot.storeInventory.map((entry) =>
      entry.store === p.subject.store
        ? { ...entry, status: 'unavailable' as const }
        : entry,
    ),
  };
  rendered.rerender(draw());
  const remove = rendered.getByRole('button', {
    name: 'Delete',
  }) as HTMLButtonElement;
  assert.equal(remove.disabled, true);
  assert.match(remove.title, /unavailable|could not be loaded/i);
  assert.equal(
    (rendered.getByRole('button', { name: 'Cancel' }) as HTMLButtonElement)
      .disabled,
    false,
  );
  ui.fireEvent.click(remove);
  assert.equal(writes, 0);
  snapshot = p.props.snapshot;
  rendered.rerender(draw());
  assert.equal(
    (rendered.getByRole('button', { name: 'Delete' }) as HTMLButtonElement)
      .disabled,
    false,
  );
  snapshot = {
    ...snapshot,
    items: snapshot.items.filter(
      (item) => item.store !== p.subject.store || item.path !== p.subject.path,
    ),
  };
  rendered.rerender(draw());
  assert.equal(
    (rendered.getByRole('button', { name: 'Delete' }) as HTMLButtonElement)
      .disabled,
    true,
  );
  assert.match(
    rendered.baseElement.textContent ?? '',
    /item is no longer available/,
  );
});

test('a retired access ticket rejects a late secret read without relying on component remount', async () => {
  const { AccessLifetime } = await import('../src/app/access-lifetime');
  const lifetime = new AccessLifetime();
  let finish!: (value: ReadItemResponse) => void;
  const response = new Promise<ReadItemResponse>((resolve) => {
    finish = resolve;
  });
  const p = await setup(() => response);
  p.props.accessTicket = lifetime.capture();
  const view = ui.render(p.draw());
  ui.fireEvent.click(view.getByRole('button', { name: 'Show' }));
  lifetime.retire('lock');
  await ui.act(async () => {
    finish({
      store: p.subject.store,
      path: p.subject.path,
      version: p.subject.version,
      value: 'retired-secret-marker',
    });
  });
  assert.equal(view.queryByText(/retired-secret-marker/), null);
});

test('losing focus remasks a revealed vault value', async () => {
  const p = await setup(async (request) => ({
    store: request.storeId,
    path: request.path,
    version: request.version,
    value: 'visible-secret-marker',
  }));
  const rendered = ui.render(p.draw());
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Show' }));
  await rendered.findByText('visible-secret-marker');

  ui.fireEvent(window, new Event('blur'));

  assert.equal(rendered.queryByText('visible-secret-marker'), null);
  assert.ok(rendered.getByText('••••••••••••'));
});

test('losing focus covers a plaintext vault edit without discarding it', async () => {
  const p = await setup(
    undefined,
    (item) => item.path === '/deploy/staging-token',
  );
  const rendered = ui.render(p.draw());
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Edit' }));
  const contents = await rendered.findByRole('textbox', { name: 'Contents' });
  ui.fireEvent.change(contents, { target: { value: 'retained private edit' } });

  ui.fireEvent(window, new Event('blur'));

  assert.equal(rendered.queryByRole('textbox', { name: 'Contents' }), null);
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Show' }));
  assert.equal(
    (rendered.getByRole('textbox', { name: 'Contents' }) as HTMLTextAreaElement)
      .value,
    'retained private edit',
  );
});

test('a plaintext edit response arriving after focus loss stays concealed', async () => {
  let finish!: (response: ReadItemResponse) => void;
  const pending = new Promise<ReadItemResponse>((resolve) => {
    finish = resolve;
  });
  const p = await setup(
    () => pending,
    (item) => item.path === '/deploy/staging-token',
  );
  const rendered = ui.render(p.draw());
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Edit' }));
  ui.fireEvent(window, new Event('blur'));

  await ui.act(async () => {
    finish({
      store: p.subject.store,
      path: p.subject.path,
      version: p.subject.version,
      value: 'late private edit',
    });
    await pending;
  });

  assert.equal(rendered.queryByText('late private edit'), null);
  assert.equal(rendered.queryByRole('textbox', { name: 'Contents' }), null);
});

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
      server.profileName === store.server
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
      server.profileName === store.server
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

test('copy failures are forwarded to the shell error handler', async () => {
  const p = await setup();
  const handled: unknown[] = [];
  p.props.bridge = {
    ...p.props.bridge,
    copyItemValue: async () => {
      throw {
        code: 'protocol',
        message: 'The agent answered out of order.',
        retryable: false,
        fatal: true,
        ambiguous: false,
      };
    },
  };
  p.props.onCommandError = (error) => {
    handled.push(error);
  };
  const r = ui.render(p.draw());
  ui.fireEvent.click(r.getAllByRole('button', { name: 'Copy' })[0]);
  await ui.waitFor(() => assert.equal(handled.length, 1));
  // No toast of the panel's own: an unsafe agent is not a warning to read past.
  assert.equal(document.querySelector('.toasts')?.textContent ?? '', '');
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
  assert.equal(password.getAttribute('placeholder'), 'password');
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

test('background version changes retain the draft and its original write precondition', async () => {
  const p = await setup(
    undefined,
    (item) => item.path === '/logins/github.com',
  );
  let submitted: Parameters<Bridge['editTextItem']>[0] | undefined;
  let conflict: { version: number; draft: string } | undefined;
  p.bridge.editTextItem = async (request) => {
    submitted = request;
    throw {
      code: 'conflict',
      message: 'The item changed.',
      retryable: false,
      fatal: false,
      ambiguous: false,
    };
  };
  p.props.onConflict = (item, draft) => {
    conflict = { version: item.version, draft };
  };
  const rendered = ui.render(p.draw());
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Edit' }));
  const username = await rendered.findByLabelText('User name');
  ui.fireEvent.change(username, { target: { value: 'retained-draft' } });
  p.props.snapshot = {
    ...p.props.snapshot,
    items: p.props.snapshot.items.map((item) =>
      item.store === p.subject.store && item.path === p.subject.path
        ? { ...item, version: item.version + 1 }
        : item,
    ),
  };
  rendered.rerender(p.draw());
  assert.equal(
    (rendered.getByLabelText('User name') as HTMLInputElement).value,
    'retained-draft',
  );
  assert.match(rendered.container.textContent ?? '', /original version/);
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Save changes' }));
  await ui.waitFor(() => assert.ok(conflict));
  assert.equal(submitted?.version, p.subject.version);
  assert.equal(conflict?.version, p.subject.version);
  assert.match(conflict.draft, /retained-draft/);
  p.props.accessGeneration = 1;
  rendered.rerender(p.draw());
  await ui.waitFor(() =>
    assert.ok(rendered.getByRole('button', { name: 'Edit' })),
  );
  assert.equal(rendered.queryByLabelText('User name'), null);
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

test('canceling the discard keeps the draft and the reader in place', async () => {
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

test('editing moves focus into the editor and returns it after Cancel', async () => {
  const p = await setup(
    undefined,
    (item) => item.path === '/logins/github.com',
  );
  const rendered = ui.render(p.draw());
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Edit' }));
  const username = await rendered.findByLabelText('User name');
  await ui.waitFor(() => assert.equal(document.activeElement, username));
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Cancel' }));
  const edit = await rendered.findByRole('button', { name: 'Edit' });
  await ui.waitFor(() => assert.equal(document.activeElement, edit));
});

test('a completed edit returns focus to the item actions', async () => {
  const p = await setup(
    undefined,
    (item) => item.path === '/logins/github.com',
  );
  p.bridge.editTextItem = async () => ({ applied: true });
  const rendered = ui.render(p.draw());
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Edit' }));
  const save = await rendered.findByRole('button', { name: 'Save changes' });
  ui.fireEvent.click(save);
  const edit = await rendered.findByRole('button', { name: 'Edit' });
  await ui.waitFor(() => assert.equal(document.activeElement, edit));
});

test('a file opens a Replace dialog while text items keep inline Edit', async () => {
  const p = await setup(undefined, (item) => item.kind === 'File');
  let replaced: Parameters<Bridge['pickAndReplaceFile']>[0] | undefined;
  p.bridge.pickAndReplaceFile = async (request) => {
    replaced = request;
    return { applied: true };
  };
  const rendered = ui.render(p.draw());
  assert.equal(rendered.queryByRole('button', { name: 'Edit' }), null);
  const replace = rendered.getByRole('button', { name: 'Replace' });
  ui.fireEvent.click(replace);
  const dialog = ui.screen.getByRole('dialog', {
    name: `Replace ${p.subject.path.split('/').at(-1)}`,
  });
  const cancel = ui.within(dialog).getByRole('button', { name: 'Cancel' });
  assert.ok(document.activeElement === cancel, 'Cancel receives initial focus');
  assert.equal(rendered.queryByRole('button', { name: 'Save changes' }), null);
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(dialog).getByRole('button', { name: 'Choose file…' }),
    );
    await Promise.resolve();
    await Promise.resolve();
  });
  assert.ok(replaced);
  assert.deepEqual(replaced, {
    storeId: p.subject.store,
    path: p.subject.path,
    version: p.subject.version,
  });
  assert.equal(ui.screen.queryByRole('dialog'), null);
  assert.ok(replace.isConnected);
});

test('a file replacement conflict closes Replace and reports replacement-specific recovery', async () => {
  const p = await setup(undefined, (item) => item.kind === 'File');
  let conflict:
    | { item: typeof p.subject; draft: string; operation?: 'edit' | 'replace' }
    | undefined;
  p.bridge.pickAndReplaceFile = async () => {
    throw {
      code: 'conflict',
      message: 'The file changed.',
      retryable: false,
      fatal: false,
      ambiguous: false,
    };
  };
  p.props.onConflict = (item, draft, operation) => {
    conflict = { item, draft, operation };
  };
  const rendered = ui.render(p.draw());
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Replace' }));
  const dialog = ui.screen.getByRole('dialog');
  await ui.act(async () => {
    ui.fireEvent.click(
      ui.within(dialog).getByRole('button', { name: 'Choose file…' }),
    );
    await Promise.resolve();
    await Promise.resolve();
  });
  assert.equal(ui.screen.queryByRole('dialog'), null);
  assert.equal(conflict?.item.path, p.subject.path);
  assert.equal(conflict?.draft, '');
  assert.equal(conflict?.operation, 'replace');
});

test('outside clicks close details while clicks inside and dialog interactions do not', async () => {
  const { props, draw } = await setup();
  let closed = 0;
  props.onClose = () => {
    closed++;
  };
  ui.render(draw());
  const panel = document.querySelector('.details')!;
  const pointer = (target: Element) =>
    ui.fireEvent(
      target,
      new window.MouseEvent('pointerdown', { bubbles: true, button: 0 }),
    );
  pointer(panel);
  assert.equal(closed, 0);
  const dialog = document.createElement('div');
  dialog.setAttribute('role', 'dialog');
  document.body.append(dialog);
  pointer(dialog);
  pointer(document.body);
  assert.equal(closed, 0);
  dialog.remove();
  pointer(document.body);
  assert.equal(closed, 1);
});
