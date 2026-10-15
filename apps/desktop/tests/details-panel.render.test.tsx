import assert from 'node:assert/strict';
import test from 'node:test';
import { PROTOCOL_CAPABILITIES } from '../src/model/types';
import { createElement, StrictMode } from 'react';
import type { Bridge, ReadItemResponse } from '../src/bridge';
import type { GuardVerdict, LocationStore } from '../src/location';
import type { Item } from '../src/model';
import type { AgentSnapshot } from '../src/model/types';
import type { DetailsPanelProps } from '../src/screens/details-panel';
import { installDom } from './lib/dom-harness';
import { loadTestShell, type TestShellProps } from './lib/test-shell';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
});
let ui: typeof import('@testing-library/react');
test.before(async () => {
  ui = await import('@testing-library/react');
});
test.afterEach(() => ui.cleanup());

async function setup(
  readItem?: Bridge['readItem'],
  selectItem?: (item: Item) => boolean,
  /** Present when the panel is mounted under a store's guard registry. */
  store?: LocationStore,
) {
  const { DetailsPanel } = await import('../src/screens/details-panel');
  const { NavigationGuardProvider } = await import('../src/navigation-guard');
  const { ToastProvider, ToastController } = await import('../kit/toasts');
  const { OverlayProvider } = await import('../kit/overlay-primitives');
  const { FIXTURE } = await import('../src/mock/fixture');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const subject = FIXTURE.items.find(
    selectItem ?? ((item) => item.path === '/env/prod/DATABASE_URL'),
  )!;
  assert.ok(subject);
  const TestShell = await loadTestShell();
  const bridge = mockBridge();
  const props: DetailsPanelProps = {
    selection: { store: subject.store, path: subject.path },
    onClose: () => {},
    onTrash: () => {},
    onConflict: () => {},
  };
  // The services the panel reads from the shell; a test replaces one before
  // the next draw.
  const shell: Omit<TestShellProps, 'children'> & {
    snapshot: AgentSnapshot;
    bridge: Bridge;
  } = {
    snapshot: FIXTURE,
    bridge: readItem ? { ...bridge, readItem } : bridge,
  };
  const panel = () =>
    createElement(OverlayProvider, {
      backgroundRef: { current: document.getElementById('root') },
      portalRoot: document.getElementById('overlays')!,
      children: createElement(ToastProvider, {
        controller: new ToastController(),
        children: createElement(TestShell, {
          ...shell,
          children: createElement(DetailsPanel, props),
        }),
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
  return { props, shell, draw, subject, bridge };
}

for (const kind of ['Secret', 'File'] as const) {
  test(`${kind} actions remain visible but disabled after vault access is lost`, async () => {
    const p = await setup(undefined, (item) => item.kind === kind);
    const rendered = ui.render(p.draw());
    p.shell.snapshot = {
      ...p.shell.snapshot,
      storeInventory: p.shell.snapshot.storeInventory.map((entry) =>
        entry.store === p.subject.store
          ? { ...entry, status: 'unavailable' as const }
          : entry,
      ),
    };
    rendered.rerender(p.draw());
    const close = rendered.getByRole('button', { name: 'Close' });
    // Favorite is a setting on this device and reads nothing from the vault.
    const favorite = rendered.getByRole('button', { name: 'Favorite' });
    const actions = rendered
      .getAllByRole('button')
      .filter(
        (button) => button !== close && button !== favorite,
      ) as HTMLButtonElement[];
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
    assert.equal((favorite as HTMLButtonElement).disabled, false);
  });
}

test('the star adds the item to Favorites, and a copy to Recent', async () => {
  window.localStorage.clear();
  const p = await setup(
    undefined,
    (item) => item.path === '/logins/github.com',
  );
  const r = ui.render(p.draw());
  const star = r.getByRole('button', { name: 'Favorite' });
  assert.equal(star.getAttribute('aria-pressed'), 'false');
  assert.equal(star.title, 'Add to favorites');
  ui.fireEvent.click(star);
  assert.equal(star.getAttribute('aria-pressed'), 'true');
  assert.equal(star.title, 'Remove from favorites');

  ui.fireEvent.click(r.getAllByRole('button', { name: 'Copy password' })[0]);
  // The toast gives the host's clear time from the copy's response.
  await ui.waitFor(() =>
    assert.match(
      document.body.textContent ?? '',
      /Password copied · clears in 30 s/,
    ),
  );
  // Both lists name the item by its store and dirent id, not its path.
  const ref = { store: 'acct:personal', dirent: p.subject.direntId };
  assert.deepEqual(
    JSON.parse(window.localStorage.getItem('foks.recentFavorites') ?? '{}'),
    { recent: [ref], favorites: [ref] },
  );

  ui.fireEvent.click(star);
  assert.equal(star.getAttribute('aria-pressed'), 'false');
  window.localStorage.clear();
});

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
  const original = p.shell.snapshot;
  const rendered = ui.render(p.draw());
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Edit' }));
  const username = await rendered.findByLabelText('User name');
  ui.fireEvent.change(username, { target: { value: 'retained-draft' } });
  p.shell.snapshot = {
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
  p.shell.snapshot = original;
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
  p.shell.snapshot = {
    ...p.shell.snapshot,
    parties: p.shell.snapshot.parties.filter(
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
  p.shell.accessNow = () => now;
  const store = p.shell.snapshot.stores.find(
    (entry) => entry.id === p.subject.store,
  )!;
  p.shell.snapshot = {
    ...p.shell.snapshot,
    observedExpiredLeases: [],
    servers: p.shell.snapshot.servers.map((server) =>
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

test('an already-open Delete permanently sheet disables its action when access is lost and re-enables after recovery', async () => {
  const p = await setup(
    undefined,
    (item) => item.path === '/logins/github.com',
  );
  const { WriteOverlay } = await import('../src/screens/write-workflows');
  const TestShell = await loadTestShell();
  const { OverlayProvider } = await import('../kit/overlay-primitives');
  const { ToastProvider, ToastController } = await import('../kit/toasts');
  let snapshot = p.shell.snapshot;
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
        children: createElement(TestShell, {
          snapshot,
          bridge: p.bridge,
          children: createElement(WriteOverlay, {
            workflow: { kind: 'delete', item: p.subject },
            setWorkflow: () => {},
            onRefreshConflict: async () => null,
            onDiscardConflict: () => {},
            onDeleteConflict: async () => {},
            onOpenExisting: async () => {},
            onRestoreTrash: async () => {},
          }),
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
    name: 'Delete permanently',
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
  snapshot = p.shell.snapshot;
  rendered.rerender(draw());
  assert.equal(
    (
      rendered.getByRole('button', {
        name: 'Delete permanently',
      }) as HTMLButtonElement
    ).disabled,
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
    (
      rendered.getByRole('button', {
        name: 'Delete permanently',
      }) as HTMLButtonElement
    ).disabled,
    true,
  );
  assert.match(
    rendered.baseElement.textContent ?? '',
    /item is no longer available/,
  );
});

test('a retired access ticket rejects a late secret read without relying on component remount', async () => {
  const { AccessLifetime } = await import('../src/data/access-lifetime');
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
  const other = p.shell.snapshot.items.find((item) => item.kind === 'Secret')!;
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

  const store = p.shell.snapshot.stores.find(
    (entry) => entry.id === p.subject.store,
  );
  assert.ok(store);
  p.shell.snapshot = {
    ...p.shell.snapshot,
    servers: p.shell.snapshot.servers.map((server) =>
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

  p.shell.snapshot = {
    ...p.shell.snapshot,
    servers: p.shell.snapshot.servers.map((server) =>
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
  // The toast host's assertive region is always present; wait for an alert
  // with text.
  await ui.waitFor(() =>
    assert.ok(r.getAllByRole('alert').some((alert) => alert.textContent)),
  );
  assert.ok(r.queryByText('/wrong-target', { exact: false }) === null);
  const before = reads;
  ui.fireEvent.click(r.getByRole('button', { name: 'Show' }));
  await ui.waitFor(() => assert.ok(reads > before));
});

test('copy failures are forwarded to the shell error handler', async () => {
  const p = await setup();
  const handled: unknown[] = [];
  p.shell.bridge = {
    ...p.shell.bridge,
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
  p.shell.commandError = (error) => {
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

  // User name, Password and Website, each masked until shown.
  assert.equal(r.getAllByText('••••••••••••').length, 3);
  ui.fireEvent.click(r.getByRole('button', { name: 'Edit' }));
  const password = await r.findByLabelText('Password');
  const username = r.getByLabelText('User name');
  assert.equal(password.getAttribute('type'), 'password');
  assert.equal(password.getAttribute('placeholder'), 'password');
  assert.equal(r.queryByRole('textbox', { name: 'Contents' }), null);

  ui.fireEvent.click(r.getByRole('button', { name: 'Show password' }));
  assert.equal(password.getAttribute('type'), 'text');
  ui.fireEvent.change(username, { target: { value: 'satoshi-next' } });
  ui.fireEvent.change(password, { target: { value: 'correct horse' } });
  ui.fireEvent.click(r.getByRole('button', { name: 'Save changes' }));

  await ui.waitFor(() => assert.match(saved, /username: satoshi-next/));
  assert.match(saved, /password: correct horse/);
  assert.match(saved, /url: https:\/\/github\.com\/login/);
});

test('the login editor generates a password into its own line', async () => {
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
  ui.fireEvent.click(r.getByRole('button', { name: 'Edit' }));
  const password = (await r.findByLabelText('Password')) as HTMLInputElement;

  // A typed password naming the site is warned about; Save stays enabled.
  ui.fireEvent.change(password, { target: { value: 'github-2024' } });
  assert.ok(r.getByText('Weak · contains the site or user name'));

  ui.fireEvent.click(r.getByRole('button', { name: 'Generate' }));
  const panel = r.getByRole('group', { name: 'Generate a password' });
  const generated =
    ui.within(panel).getByLabelText('Generated password').textContent ?? '';
  assert.ok(generated.length >= 8);
  ui.fireEvent.click(
    ui.within(panel).getByRole('button', { name: 'Use password' }),
  );
  assert.equal(password.value, generated);
  // The generated value is still masked until Show.
  assert.equal(password.type, 'password');
  assert.match(r.getByText(/ bits$/).textContent ?? '', /^(Good|Strong) · /);

  ui.fireEvent.click(r.getByRole('button', { name: 'Save changes' }));
  await ui.waitFor(() => assert.ok(saved));
  const lines = saved.split('\n');
  assert.equal(lines.filter((line) => line.startsWith('password: ')).length, 1);
  assert.ok(lines.includes(`password: ${generated}`));
  assert.ok(lines.includes('url: https://github.com/login'));
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
  p.shell.snapshot = {
    ...p.shell.snapshot,
    items: p.shell.snapshot.items.map((item) =>
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

test('background identity changes preserve the original edit target', async () => {
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
  p.shell.snapshot = {
    ...p.shell.snapshot,
    items: p.shell.snapshot.items.map((item) =>
      item.store === p.subject.store && item.path === p.subject.path
        ? { ...item, direntId: 'ab'.repeat(16) }
        : item,
    ),
  };
  rendered.rerender(p.draw());
  assert.equal(
    (rendered.getByLabelText('User name') as HTMLInputElement).value,
    'retained-draft',
  );
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Save changes' }));
  await ui.waitFor(() => assert.ok(conflict));
  assert.equal(submitted?.version, p.subject.version);
  assert.equal(submitted?.direntId, p.subject.direntId);
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
  const { LocationStore } = await import('../src/location');
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
  const other = p.shell.snapshot.items.find(
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
    direntId: p.subject.direntId,
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

test('a login lists each field masked and shows only the field asked for', async () => {
  let reads = 0;
  const p = await setup(
    undefined,
    (item) => item.path === '/logins/github.com',
  );
  p.shell.bridge = {
    ...p.shell.bridge,
    readItem: async (request) => {
      reads++;
      return p.bridge.readItem(request);
    },
  };
  const r = ui.render(p.draw());
  assert.equal(r.getAllByText('••••••••••••').length, 3);
  ui.fireEvent.click(r.getByRole('button', { name: 'Show user name' }));
  await r.findByText('satoshi');
  // The password and website stay masked although the text was read.
  assert.equal(r.getAllByText('••••••••••••').length, 2);
  assert.equal(r.queryByText('hx7-Qm2!vTe9-pale-orbit'), null);
  ui.fireEvent.click(r.getByRole('button', { name: 'Show password' }));
  assert.ok(r.getByText('hx7-Qm2!vTe9-pale-orbit'));
  assert.equal(reads, 1);
  ui.fireEvent.click(r.getByRole('button', { name: 'Hide user name' }));
  ui.fireEvent.click(r.getByRole('button', { name: 'Hide password' }));
  assert.equal(r.getAllByText('••••••••••••').length, 3);
  // Hiding the last shown field drops the text; a later Show reads again.
  ui.fireEvent.click(r.getByRole('button', { name: 'Show website' }));
  await r.findByText('https://github.com/login');
  assert.equal(reads, 2);
});

test('login copies put only the chosen field on the clipboard', async () => {
  const p = await setup(
    undefined,
    (item) => item.path === '/logins/github.com',
  );
  const { mockCopies } = await import('../src/mock/mock-bridge');
  const before = mockCopies.length;
  const r = ui.render(p.draw());
  ui.fireEvent.click(r.getByRole('button', { name: 'Copy user name' }));
  await ui.waitFor(() =>
    assert.match(document.body.textContent ?? '', /User name copied/),
  );
  // The main action and the row button both copy the password alone.
  ui.fireEvent.click(r.getAllByRole('button', { name: 'Copy password' })[0]);
  ui.fireEvent.click(r.getAllByRole('button', { name: 'Copy password' })[1]);
  ui.fireEvent.click(r.getByRole('button', { name: 'Copy website' }));
  await ui.waitFor(() => assert.equal(mockCopies.length - before, 4));
  assert.deepEqual(
    mockCopies.slice(before).map(({ field, text }) => [field, text]),
    [
      ['username', 'satoshi'],
      ['password', 'hx7-Qm2!vTe9-pale-orbit'],
      ['password', 'hx7-Qm2!vTe9-pale-orbit'],
      ['url', 'https://github.com/login'],
    ],
  );
  assert.match(document.body.textContent ?? '', /Password copied/);
});

test('a field the login lacks is reported, not copied, and reads as not set', async () => {
  const p = await setup(
    undefined,
    (item) => item.path === '/logins/fastmail.com',
  );
  const { mockCopies } = await import('../src/mock/mock-bridge');
  const before = mockCopies.length;
  const handled: unknown[] = [];
  p.shell.commandError = (error) => {
    handled.push(error);
  };
  const r = ui.render(p.draw());
  ui.fireEvent.click(r.getByRole('button', { name: 'Copy website' }));
  await ui.waitFor(() =>
    assert.match(document.body.textContent ?? '', /This item has no website\./),
  );
  assert.equal(mockCopies.length, before);
  assert.equal(handled.length, 0);
  ui.fireEvent.click(r.getByRole('button', { name: 'Show website' }));
  await r.findByText('Not set');
  const copy = r.getByRole('button', { name: 'Copy website' });
  assert.equal((copy as HTMLButtonElement).disabled, true);
  assert.equal(copy.getAttribute('title'), 'No website saved');
});

test('a login saved outside /logins/ is a Password by its listed shape', async () => {
  const p = await setup();
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const login = {
    ...p.subject,
    path: '/env/prod/grafana',
    direntId: '0000000000000000000000000000001f',
    shape: 'login' as const,
  };
  const snapshot = {
    ...p.shell.snapshot,
    items: [...p.shell.snapshot.items, login],
    plaintext: {
      ...p.shell.snapshot.plaintext,
      [`${login.store}|${login.path}`]:
        'user: admin\npassword: graf-77\nurl: https://grafana.internal',
    },
  };
  let saved = '';
  p.shell.snapshot = snapshot;
  p.shell.bridge = {
    ...mockBridge(snapshot),
    editTextItem: async (request) => {
      saved = request.value;
      return { applied: true };
    },
  };
  p.props.selection = { store: login.store, path: login.path };
  const r = ui.render(p.draw());
  assert.ok(r.getAllByRole('button', { name: 'Copy password' }).length);
  assert.equal(r.queryByRole('button', { name: 'Copy' }), null);
  // The structured editor covers it as it covers a login under /logins/.
  ui.fireEvent.click(r.getByRole('button', { name: 'Edit' }));
  const username = await r.findByLabelText('User name');
  assert.equal((username as HTMLInputElement).value, 'admin');
  ui.fireEvent.change(username, { target: { value: 'grafana-admin' } });
  ui.fireEvent.click(r.getByRole('button', { name: 'Save changes' }));
  await ui.waitFor(() => assert.match(saved, /^user: grafana-admin$/m));
  assert.match(saved, /^password: graf-77$/m);
});

/**
 * Mounts the panel on one extra login with `text` as its stored value. An
 * unshaped login is listed as an agent that predates the shape field lists
 * it, so only its `/logins/` path makes it a Password.
 */
async function setupLogin(
  text: string,
  { path = '/wifi/cabin', shaped = true } = {},
) {
  const p = await setup();
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const login = {
    ...p.subject,
    path,
    direntId: '0000000000000000000000000000002f',
    shape: 'login' as const,
  };
  if (!shaped) delete (login as Partial<typeof login>).shape;
  const snapshot = {
    ...p.shell.snapshot,
    items: [...p.shell.snapshot.items, login],
    plaintext: {
      ...p.shell.snapshot.plaintext,
      [`${login.store}|${login.path}`]: text,
    },
  };
  const saved: string[] = [];
  p.shell.snapshot = snapshot;
  p.shell.bridge = {
    ...mockBridge(snapshot),
    editTextItem: async (request) => {
      saved.push(request.value);
      return { applied: true };
    },
  };
  p.props.selection = { store: login.store, path: login.path };
  return { ...p, saved };
}

test('the login editor masks the line Copy reads as the password and keeps other lines as stored', async () => {
  const p = await setupLogin('ssid: Cabin\r\npassword:x \r\n');
  const r = ui.render(p.draw());
  ui.fireEvent.click(r.getByRole('button', { name: 'Edit' }));
  const password = await r.findByLabelText('Password');
  assert.equal(password.getAttribute('type'), 'password');
  assert.equal((password as HTMLInputElement).value, 'x ');
  ui.fireEvent.click(r.getByRole('button', { name: 'Show password' }));
  assert.equal(password.getAttribute('type'), 'text');

  // Losing focus covers every row, not only the password.
  ui.fireEvent(window, new Event('blur'));
  assert.equal(r.queryByLabelText('Network'), null);
  assert.equal(r.queryByLabelText('Password'), null);
  ui.fireEvent.click(r.getByRole('button', { name: 'Show' }));
  assert.equal(r.getByLabelText('Password').getAttribute('type'), 'password');

  // Each line keeps its own separator.
  ui.fireEvent.change(r.getByLabelText('Network'), {
    target: { value: 'Cabin-5G' },
  });
  ui.fireEvent.change(r.getByLabelText('Password'), {
    target: { value: 'y ' },
  });
  ui.fireEvent.click(r.getByRole('button', { name: 'Save changes' }));
  await ui.waitFor(() => assert.equal(p.saved.length, 1));
  assert.equal(p.saved[0], 'ssid: Cabin-5G\r\npassword:y \r\n');
});

test('a Password whose text has no password line edits in the concealable textarea', async () => {
  const p = await setupLogin('ssid: Cabin\nnote: no password yet');
  const r = ui.render(p.draw());
  ui.fireEvent.click(r.getByRole('button', { name: 'Edit' }));
  const contents = await r.findByRole('textbox', { name: 'Contents' });
  assert.equal(
    (contents as HTMLTextAreaElement).value,
    'ssid: Cabin\nnote: no password yet',
  );
  // The editor is chosen when the edit opens; typing a password line keeps
  // the textarea, and its focus, in place.
  contents.focus();
  ui.fireEvent.change(contents, {
    target: { value: 'ssid: Cabin\npassword: x' },
  });
  assert.equal(r.getByRole('textbox', { name: 'Contents' }), contents);
  assert.equal(document.activeElement, contents);
  assert.equal(r.queryByLabelText('Password'), null);
  ui.fireEvent(window, new Event('blur'));
  assert.equal(r.queryByRole('textbox', { name: 'Contents' }), null);
});

test('a login line keyed by an Object.prototype name is listed and edited under its key', async () => {
  const p = await setupLogin(
    'user: a\npassword: b\nconstructor: x\ntoString: y',
  );
  const r = ui.render(p.draw());
  ui.fireEvent.click(r.getByRole('button', { name: 'Show user name' }));
  await r.findByText('a');
  ui.fireEvent.click(r.getByRole('button', { name: 'Show constructor' }));
  assert.ok(r.getByText('x'));
  ui.fireEvent.click(r.getByRole('button', { name: 'Show tostring' }));
  assert.ok(r.getByText('y'));
  ui.fireEvent.click(r.getByRole('button', { name: 'Edit' }));
  const constructorField = await r.findByLabelText('constructor');
  assert.equal((constructorField as HTMLInputElement).value, 'x');
  assert.equal((r.getByLabelText('toString') as HTMLInputElement).value, 'y');
});

test('the login editor lists no blank lines and saves them as stored', async () => {
  const p = await setupLogin('user: a\n\npassword: b\nurl: c\n');
  const r = ui.render(p.draw());
  ui.fireEvent.click(r.getByRole('button', { name: 'Edit' }));
  const username = await r.findByLabelText('User name');
  assert.equal(r.queryByLabelText('Value'), null);
  assert.equal(r.getAllByRole('textbox').length, 2);
  ui.fireEvent.change(username, { target: { value: 'z' } });
  ui.fireEvent.click(r.getByRole('button', { name: 'Save changes' }));
  await ui.waitFor(() => assert.equal(p.saved.length, 1));
  assert.equal(p.saved[0], 'user: z\n\npassword: b\nurl: c\n');
});

test('a Password known only by its /logins/ path also copies its whole value', async () => {
  const p = await setupLogin('hunter2', {
    path: '/logins/legacy.example',
    shaped: false,
  });
  const { mockCopies } = await import('../src/mock/mock-bridge');
  const before = mockCopies.length;
  const r = ui.render(p.draw());
  assert.ok(r.getAllByRole('button', { name: 'Copy password' }).length);
  ui.fireEvent.click(r.getByRole('button', { name: 'Copy value' }));
  await ui.waitFor(() => assert.equal(mockCopies.length - before, 1));
  assert.deepEqual(
    mockCopies.slice(before).map(({ field, text }) => [field, text]),
    [['value', 'hunter2']],
  );
  assert.match(document.body.textContent ?? '', /Value copied/);
});

test('a Password with a listed shape offers no whole-value copy', async () => {
  const p = await setupLogin('user: a\npassword: b', {
    path: '/logins/current.example',
  });
  const r = ui.render(p.draw());
  assert.ok(r.getAllByRole('button', { name: 'Copy password' }).length);
  assert.equal(r.queryByRole('button', { name: 'Copy value' }), null);
});
