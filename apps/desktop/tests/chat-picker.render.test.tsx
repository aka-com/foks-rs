import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, StrictMode, useState } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import { installDom } from './lib/dom-harness';
import { readSource } from './lib/source';
import type { Bridge } from '../src/bridge';
import type { AgentSnapshot, TeamStore } from '../src/model';
import type { ChatReply } from '../src/chat-contract';
import type { Location, NavigateOptions } from '../src/location';

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
test.after(async () => {
  await vite.close();
});

test('the picker reserves vertical focus-ring clearance inside the scrolling sheet body', async () => {
  const css = await readSource(
    '../src/screens/chat-picker.css',
    import.meta.url,
  );
  assert.match(css, /\.chat-picker\s*\{[^}]*padding-block: 4px;/);
});

async function mount(override?: (base: Bridge) => Bridge, tabbed = false) {
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const snapshot: AgentSnapshot = {
    ...FIXTURE,
    servers: FIXTURE.servers.map(
      (server: AgentSnapshot['servers'][number]) => ({
        ...server,
        services: { ...server.services, chat: true },
        compatibility: { status: 'not-required' },
      }),
    ),
  };
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const { ChatInboxProvider } = await vite.ssrLoadModule(
    '/src/chat/inbox-provider.tsx',
  );
  const { NewChatSheet } = await vite.ssrLoadModule(
    '/src/screens/chat-new.tsx',
  );
  const { OverlayProvider } = await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  );
  const locations = (await vite.ssrLoadModule(
    '/src/location.ts',
  )) as typeof import('../src/location');
  const { NavigationGuardProvider } = await vite.ssrLoadModule(
    '/src/navigation-guard.tsx',
  );
  const { ChatTab } = await vite.ssrLoadModule('/src/screens/chat-tab.tsx');
  const { ChannelCreationCompletions } = await vite.ssrLoadModule(
    '/src/chat/channel-creation-provider.tsx',
  );
  const navigation = new locations.LocationStore();
  const base: Bridge = mockBridge(snapshot);
  const bridge = override?.(base) ?? base;
  const opened: { store: string; channel: string }[] = [];
  const team = snapshot.stores.find(
    (store): store is TeamStore =>
      store.kind === 'team' && store.name === 'Household',
  )!;
  navigation.navigate({ kind: 'chat', ref: team.id }, { force: true });
  function TabPane() {
    const { location } = locations.useLocationState(navigation);
    return location.kind === 'chat'
      ? createElement(ChatTab, {
          bridge,
          snapshot,
          location,
          onNavigate: (next: Location, options?: NavigateOptions) => {
            if (next.kind === 'chat' && next.ref && next.channel)
              opened.push({ store: next.ref, channel: next.channel });
            navigation.navigate(next, options);
          },
        })
      : createElement('p', null, 'Files tab');
  }
  function Pane() {
    const [shown, setShown] = useState(true);
    return createElement(
      'div',
      null,
      createElement('button', { onClick: () => setShown(true) }, 'Show picker'),
      createElement(ChannelCreationCompletions, {
        onOpen: (store: string, channel: string) => {
          opened.push({ store, channel });
        },
      }),
      shown &&
        createElement(NewChatSheet, {
          bridge,
          snapshot,
          onClose: () => setShown(false),
          onOpen: (store: string, channel: string) => {
            opened.push({ store, channel });
            setShown(false);
          },
        }),
    );
  }
  ui.render(
    createElement(
      StrictMode,
      null,
      createElement(OverlayProvider, {
        backgroundRef: { current: null },
        portalRoot: document.getElementById('overlays'),
        children: createElement(ChatInboxProvider, {
          bridge,
          snapshot,
          children: createElement(NavigationGuardProvider, {
            store: navigation,
            children: createElement(tabbed ? TabPane : Pane),
          }),
        }),
      }),
    ),
  );
  if (tabbed) await ui.screen.findByRole('button', { name: 'New chat' });
  else await ui.screen.findByRole('button', { name: /Household · #general/ });
  return { opened, team, navigation };
}

async function form(team: TeamStore) {
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Create channel' }));
  ui.fireEvent.change(ui.screen.getByRole('combobox', { name: 'Team' }), {
    target: { value: team.id },
  });
  ui.fireEvent.change(
    ui.screen.getByRole('textbox', { name: 'Channel name' }),
    { target: { value: 'design' } },
  );
  await ui.waitFor(() =>
    assert.equal(
      ui.screen.getByRole<HTMLButtonElement>('button', {
        name: 'Create channel',
      }).disabled,
      false,
    ),
  );
}

test('one searchable picker opens a conversation across teams with no confirmation steps', async () => {
  const { opened, team } = await mount();
  assert.equal(ui.screen.queryByRole('button', { name: 'Continue' }), null);
  assert.equal(ui.screen.queryByRole('button', { name: 'Open chat' }), null);
  ui.fireEvent.change(
    ui.screen.getByRole('searchbox', { name: 'Search conversations' }),
    { target: { value: 'household' } },
  );
  assert.equal(
    ui.screen.queryByRole('button', { name: /Engineering · #general/ }),
    null,
  );
  ui.fireEvent.click(
    ui.screen.getByRole('button', { name: /Household · #general/ }),
  );
  assert.equal(opened.length, 1);
  assert.equal(opened[0].store, team.id);
  assert.equal(ui.screen.queryByRole('dialog'), null);
});

test('creation is one team/name/description/audience form with protocol validation', async () => {
  const { team, opened } = await mount();
  await form(team);
  assert.ok(ui.screen.getByRole('textbox', { name: 'Channel description' }));
  assert.ok(ui.screen.getByRole('radiogroup', { name: 'Channel audience' }));
  ui.fireEvent.change(
    ui.screen.getByRole('textbox', { name: 'Channel name' }),
    { target: { value: 'general' } },
  );
  assert.equal(
    ui.screen.getByRole<HTMLButtonElement>('button', { name: 'Create channel' })
      .disabled,
    true,
  );
  ui.fireEvent.change(
    ui.screen.getByRole('textbox', { name: 'Channel name' }),
    { target: { value: 'design' } },
  );
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Create channel' }));
  await ui.waitFor(() => assert.equal(opened.length, 1));
  assert.equal(opened[0].store, team.id);
});

test('closing an in-flight creation keeps application-owned work and never redirects later', async () => {
  let finish!: () => void;
  let submitted = 0;
  const { team, opened } = await mount((base) => ({
    ...base,
    chat: async (store, action, view) => {
      if (action.action === 'prepare-channel') submitted++;
      if (action.action === 'attempt')
        await new Promise<void>((resolve) => {
          finish = resolve;
        });
      return base.chat(store, action, view);
    },
  }));
  await form(team);
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Create channel' }));
  await ui.waitFor(() => assert.ok(finish));
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Close' }));
  assert.equal(ui.screen.queryByRole('dialog'), null);
  await ui.act(async () => {
    finish();
  });
  await ui.waitFor(() => assert.equal(submitted, 1));
  assert.equal(opened.length, 0);
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Show picker' }));
  await ui.screen.findByRole('button', { name: /Household · #design/ });
  assert.equal(opened.length, 0);
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Cancel' }));
  const completion = ui.screen.getByRole('region', {
    name: 'Created channels',
  });
  ui.fireEvent.click(
    ui.within(completion).getByRole('button', { name: 'Open channel' }),
  );
  assert.equal(opened.length, 1);
  assert.equal(opened[0].store, team.id);
  assert.equal(
    ui.screen.queryByRole('region', { name: 'Created channels' }),
    null,
  );
});

test('uncertain creation reopens original inputs and Check again does not create a duplicate', async () => {
  let prepared: ChatReply | undefined;
  let attempts = 0;
  const { team, opened } = await mount((base) => ({
    ...base,
    chat: async (store, action, view) => {
      if (action.action === 'attempt') {
        attempts++;
        assert.ok(prepared && prepared.result.kind === 'operation');
        return {
          ...prepared,
          result: {
            kind: 'operation',
            operation: { ...prepared.result.operation, state: 'uncertain' },
          },
        };
      }
      if (action.action === 'reconcile') {
        assert.ok(prepared && prepared.result.kind === 'operation');
        return base.chat(
          store,
          { action: 'attempt', operation: prepared.result.operation.id },
          view,
        );
      }
      const reply = await base.chat(store, action, view);
      if (action.action === 'prepare-channel') prepared = reply;
      return reply;
    },
  }));
  await form(team);
  ui.fireEvent.change(
    ui.screen.getByRole('textbox', { name: 'Channel description' }),
    { target: { value: 'design discussion' } },
  );
  ui.fireEvent.click(
    ui.screen.getByRole('radio', { name: /Admins and owners/ }),
  );
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Create channel' }));
  await ui.screen.findByRole('button', { name: 'Check again' });
  assert.equal(opened.length, 0);
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Close' }));
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Show picker' }));
  ui.fireEvent.click(
    await ui.screen.findByRole('button', { name: /Household · #design/ }),
  );
  assert.equal(
    ui.screen.getByRole<HTMLInputElement>('textbox', { name: 'Channel name' })
      .value,
    'design',
  );
  assert.equal(
    ui.screen.getByRole<HTMLTextAreaElement>('textbox', {
      name: 'Channel description',
    }).value,
    'design discussion',
  );
  assert.equal(
    ui.screen
      .getByRole('radio', { name: /Admins and owners/ })
      .getAttribute('aria-checked'),
    'true',
  );
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Check again' }));
  await ui.waitFor(() => assert.equal(opened.length, 1));
  assert.equal(attempts, 1);
});

test('unsubmitted name, description, audience and chosen team survive a rail tab round trip', async () => {
  const { team, navigation } = await mount(undefined, true);
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'New chat' }));
  await form(team);
  ui.fireEvent.change(
    ui.screen.getByRole('textbox', { name: 'Channel description' }),
    { target: { value: 'unsubmitted details' } },
  );
  ui.fireEvent.click(
    ui.screen.getByRole('radio', { name: /Admins and owners/ }),
  );
  await ui.act(async () => {
    navigation.navigateTab('files');
  });
  assert.equal(ui.screen.queryByRole('dialog'), null);
  await ui.act(async () => {
    navigation.navigateTab('chat');
  });
  assert.equal(
    ui.screen.getByRole<HTMLInputElement>('textbox', { name: 'Channel name' })
      .value,
    'design',
  );
  assert.equal(
    ui.screen.getByRole<HTMLTextAreaElement>('textbox', {
      name: 'Channel description',
    }).value,
    'unsubmitted details',
  );
  assert.equal(
    ui.screen.getByRole<HTMLSelectElement>('combobox', { name: 'Team' }).value,
    team.id,
  );
  assert.equal(
    ui.screen
      .getByRole('radio', { name: /Admins and owners/ })
      .getAttribute('aria-checked'),
    'true',
  );
});

test('submitted work never restores a redirect or raw draft after changing rail tabs', async () => {
  let finish!: () => void;
  const { team, opened, navigation } = await mount(
    (base) => ({
      ...base,
      chat: async (store, action, view) => {
        if (action.action === 'attempt')
          await new Promise<void>((resolve) => {
            finish = resolve;
          });
        return base.chat(store, action, view);
      },
    }),
    true,
  );
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'New chat' }));
  await form(team);
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Create channel' }));
  await ui.waitFor(() => assert.ok(finish));
  assert.equal(navigation.getSnapshot().sheet?.['channel.form'], null);
  assert.equal(navigation.getSnapshot().sheet?.['chat.sheet'], null);
  await ui.act(async () => {
    navigation.navigateTab('files');
  });
  assert.equal(ui.screen.queryByRole('dialog'), null);
  await ui.act(async () => {
    finish();
  });
  await ui.act(async () => {
    navigation.navigateTab('chat');
  });
  const completion = await ui.screen.findByRole('region', {
    name: 'Created channels',
  });
  assert.equal(ui.screen.queryByRole('dialog'), null);
  assert.equal(opened.length, 0);
  ui.fireEvent.click(
    ui.within(completion).getByRole('button', { name: 'Dismiss' }),
  );
  assert.equal(
    ui.screen.queryByRole('region', { name: 'Created channels' }),
    null,
  );
  assert.equal(opened.length, 0);
});

test('Check again only reconciles; Retry creation explicitly delivers a saved prepared channel', async () => {
  let prepared: ChatReply | undefined;
  let attempts = 0;
  const { team, opened } = await mount((base) => ({
    ...base,
    chat: async (store, action, view) => {
      if (action.action === 'attempt') {
        attempts++;
        if (attempts === 1) {
          assert.ok(prepared && prepared.result.kind === 'operation');
          return {
            ...prepared,
            result: {
              kind: 'operation',
              operation: { ...prepared.result.operation, state: 'uncertain' },
            },
          };
        }
      }
      if (action.action === 'reconcile') {
        assert.ok(prepared);
        return prepared;
      }
      const reply = await base.chat(store, action, view);
      if (action.action === 'prepare-channel') prepared = reply;
      return reply;
    },
  }));
  await form(team);
  ui.fireEvent.click(ui.screen.getByRole('button', { name: 'Create channel' }));
  ui.fireEvent.click(
    await ui.screen.findByRole('button', { name: 'Check again' }),
  );
  const retry = await ui.screen.findByRole('button', {
    name: 'Retry creation',
  });
  assert.equal(attempts, 1);
  assert.equal(opened.length, 0);
  ui.fireEvent.click(retry);
  await ui.waitFor(() => assert.equal(opened.length, 1));
  assert.equal(attempts, 2);
});

test('standalone creation consumers explicitly report the missing application owner', async () => {
  const { useChannelCreation } = (await vite.ssrLoadModule(
    '/src/chat/channel-creation-provider.tsx',
  )) as typeof import('../src/chat/channel-creation-provider');
  function Consumer() {
    const { controller, creations } = useChannelCreation();
    assert.equal(controller, null);
    assert.deepEqual(creations, []);
    return null;
  }
  ui.render(createElement(Consumer));
});
