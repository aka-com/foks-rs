/**
 * The navigation guards the chat workflows answer with: the composer's unsent
 * message, the New chat sheet's half-finished channel, and the one move the
 * tab makes on its own behalf.
 *
 * The host is the shell's wiring in miniature — a real `LocationStore`, the
 * prompter that turns a `prompt` verdict into the confirmation, and the
 * refusal handler that collects what a `refuse` verdict says — with the Chat
 * tab under it, so each guard is exercised where the reader meets it.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import {
  createElement,
  StrictMode,
  useCallback,
  useEffect,
  useRef,
  useState,
} from 'react';
import type { ReactNode } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import { installDom } from './lib/dom-harness';
import type { Bridge } from '../src/bridge';
import type {
  ChatChannel,
  ChatConversation,
  ChatReply,
} from '../src/chat-contract';
import type { GuardVerdict, Location, LocationStore } from '../src/location';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
  act: true,
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

/** Where these tests start: the Chat tab with Engineering's conversation. */
const IN_CHAT: Location = {
  kind: 'chat',
  ref: 'team:eng',
  channel: 'ab'.repeat(16),
};

/** Engineering's second channel, so a switch inside one team is a real move. */
const DESIGN_ID = 'ef'.repeat(16);
const DESIGN: ChatChannel = {
  id: DESIGN_ID,
  name: 'design',
  description: 'Where the drawings are.',
  admin: false,
  readable: true,
  writable: true,
  read_role: 'Member (0)',
  write_role: 'Member (0)',
};
const DESIGN_CONVERSATION: ChatConversation = {
  channel: DESIGN,
  inbox_version: '1',
  read_through: '0',
  pending_read: null,
  unread: '0',
  hidden: false,
  muted: false,
  preview: null,
};

/** Lists that second channel for as long as `listed` says the team has it. */
function withDesign(base: Bridge, listed: () => boolean): Bridge {
  return {
    ...base,
    chat: async (store, action, view) => {
      const reply = await base.chat(store, action, view);
      if (!listed()) return reply;
      const result = reply.result;
      if (result.kind === 'channels')
        return {
          ...reply,
          result: { ...result, channels: [...result.channels, DESIGN] },
        };
      if (result.kind === 'inbox')
        return {
          ...reply,
          result: {
            ...result,
            channels: [...result.channels, DESIGN],
            conversations: [...result.conversations, DESIGN_CONVERSATION],
          },
        };
      return reply;
    },
  };
}

interface Pending {
  verdict: Extract<GuardVerdict, { verdict: 'prompt' }>;
  settle: (confirmed: boolean) => void;
}

interface Harness {
  store: LocationStore;
  /** Every reason a `refuse` verdict has given, in the order they were given. */
  refusals: string[];
}

async function setup(
  options: {
    override?: (base: Bridge) => Bridge;
    start?: Location;
    /** Registered before the first render, as a screen already on the page. */
    guard?: GuardVerdict;
    waitForHistory?: boolean;
    includeHousehold?: boolean;
  } = {},
): Promise<Harness> {
  const { ChatTab } = await vite.ssrLoadModule('/src/screens/chat-tab.tsx');
  const { ChatInboxProvider } = await vite.ssrLoadModule(
    '/src/chat/inbox-provider.tsx',
  );
  const locations = (await vite.ssrLoadModule(
    '/src/location.ts',
  )) as typeof import('../src/location');
  const { NavigationGuardProvider, NavigationPrompt } =
    (await vite.ssrLoadModule(
      '/src/navigation-guard.tsx',
    )) as typeof import('../src/navigation-guard');
  const { OverlayProvider } = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');

  const snapshot = {
    ...FIXTURE,
    stores: FIXTURE.stores.filter(
      (s) =>
        s.kind !== 'team' ||
        s.id === 'team:eng' ||
        (options.includeHousehold && s.id === 'team:household'),
    ),
    servers: FIXTURE.servers.map((s) =>
      s.id === 'acme' ? { ...s, services: { ...s.services, chat: true } } : s,
    ),
  };
  const bridge = options.override
    ? options.override(mockBridge(snapshot))
    : mockBridge(snapshot);
  const store = new locations.LocationStore();
  const refusals: string[] = [];
  store.setRefusalHandler((reason) => refusals.push(reason));
  store.navigate(options.start ?? IN_CHAT, { force: true });
  if (options.guard) store.registerGuard(() => options.guard ?? null);
  const portal = document.getElementById('overlays');
  if (!portal) throw new Error('missing overlay root');
  const portalRoot = portal;

  function Host(): ReactNode {
    const state = locations.useLocationState(store);
    const [prompt, setPrompt] = useState<Pending | null>(null);
    const promptRef = useRef<Pending | null>(null);
    const settle = useCallback((confirmed: boolean) => {
      const open = promptRef.current;
      if (!open) return;
      promptRef.current = null;
      setPrompt(null);
      open.settle(confirmed);
    }, []);
    useEffect(() => {
      store.setPrompter(
        (asked) =>
          new Promise<boolean>((resolve) => {
            const next = { verdict: asked, settle: resolve };
            promptRef.current = next;
            setPrompt(next);
          }),
      );
      return () => store.setPrompter(null);
    }, []);
    return createElement(
      StrictMode,
      null,
      createElement(OverlayProvider, {
        backgroundRef: { current: null },
        portalRoot,
        children: createElement(NavigationGuardProvider, {
          store,
          children: [
            createElement(ChatInboxProvider, {
              key: 'chat',
              bridge,
              snapshot,
              children:
                state.location.kind === 'chat'
                  ? createElement(ChatTab, {
                      snapshot,
                      bridge,
                      location: state.location,
                      onNavigate: (
                        next: Location,
                        navigateOptions?: import('../src/location').NavigateOptions,
                      ) => store.navigate(next, navigateOptions),
                    })
                  : createElement('p', null, 'Elsewhere'),
            }),
            prompt
              ? createElement(NavigationPrompt, {
                  key: 'prompt',
                  verdict: prompt.verdict,
                  onConfirm: () => settle(true),
                  onCancel: () => settle(false),
                })
              : null,
          ],
        }),
      }),
    );
  }

  ui.render(createElement(Host));
  if (options.waitForHistory !== false)
    await ui.screen.findByText('Team chat is ready.');
  return { store, refusals };
}

/** The confirmation's panel, or `null` when nothing is up. */
function dialog(): HTMLElement | null {
  return document.querySelector<HTMLElement>('[role="alertdialog"]');
}

function dialogButton(label: string): HTMLButtonElement {
  const found = [...document.querySelectorAll<HTMLButtonElement>('.ft .btn')];
  const target = found.find((node) => node.textContent === label);
  assert.ok(target, `the dialog offers ${label}`);
  return target;
}

/** Asks for a move the guards are consulted about, and lets a dialog mount. */
async function leave(
  store: LocationStore,
  location: Location = { kind: 'files' },
): Promise<void> {
  await ui.act(async () => {
    store.navigate(location);
    await Promise.resolve();
  });
}

async function click(node: HTMLElement): Promise<void> {
  await ui.act(async () => {
    ui.fireEvent.click(node);
    await Promise.resolve();
  });
}

function composer(): HTMLTextAreaElement {
  return ui.screen.getByRole<HTMLTextAreaElement>('textbox', {
    name: 'Message',
  });
}

function write(text: string): void {
  ui.fireEvent.change(composer(), { target: { value: text } });
}

/** The channel titles the column lists under Engineering. */
function engineeringChannels(): string[] {
  const group = document.querySelector<HTMLElement>(
    '.chat-channel-list[aria-label="Engineering"]',
  );
  return [...(group?.querySelectorAll('.chat-channel .n') ?? [])].map(
    (node) => node.textContent ?? '',
  );
}

/** A channel row of the column, by the title it draws. */
async function channelRow(title: string): Promise<HTMLButtonElement> {
  return ui.waitFor(() => {
    const row = [
      ...document.querySelectorAll<HTMLButtonElement>('.chat-channel'),
    ].find((node) => node.querySelector('.n')?.textContent === title);
    assert.ok(row, `the column lists ${title}`);
    return row;
  });
}

/** The create-a-channel form, reached the way the tab offers it. */
async function openChannelSheet(): Promise<HTMLInputElement> {
  ui.fireEvent.click(
    ui.screen.getAllByRole('button', { name: 'New channel' })[0],
  );
  await click(ui.screen.getByRole('button', { name: 'Team' }));
  await click(await ui.screen.findByRole('option', { name: /^Engineering/ }));
  return ui.screen.getByRole<HTMLInputElement>('textbox', {
    name: 'Channel name',
  });
}

test('an empty composer lets a move out of chat through', async () => {
  const { store, refusals } = await setup();
  await leave(store);
  assert.equal(dialog(), null);
  assert.deepEqual(refusals, []);
  assert.deepEqual(store.getSnapshot().location, { kind: 'files' });
});

test('an unsent message survives leaving chat without a discard prompt', async () => {
  const { store, refusals } = await setup();
  write('half a thought');
  await leave(store);
  assert.equal(dialog(), null);
  assert.deepEqual(refusals, []);
  assert.deepEqual(store.getSnapshot().location, { kind: 'files' });
  assert.equal(ui.screen.queryByRole('textbox', { name: 'Message' }), null);
  await leave(store, IN_CHAT);
  await ui.waitFor(() => assert.equal(composer().value, 'half a thought'));
});

test('switching teams preserves independent drafts without a prompt', async () => {
  const { store, refusals } = await setup({ includeHousehold: true });
  const household: Location = {
    kind: 'chat',
    ref: 'team:household',
    channel: 'ab'.repeat(16),
  };
  write('Engineering draft');
  await leave(store, household);
  assert.equal(dialog(), null);
  assert.deepEqual(store.getSnapshot().location, household);
  await ui.waitFor(() => assert.equal(composer().value, ''));
  write('Household draft');
  await leave(store, IN_CHAT);
  await ui.waitFor(() => assert.equal(composer().value, 'Engineering draft'));
  await leave(store, household);
  await ui.waitFor(() => assert.equal(composer().value, 'Household draft'));
  assert.equal(dialog(), null);
  assert.deepEqual(refusals, []);
});

test('an unsent message follows the reader between channels of one team', async () => {
  const { store, refusals } = await setup({
    override: (base) => withDesign(base, () => true),
  });
  write('half a thought');
  // A channel of the same team is a navigation like any other; the draft is
  // kept above the thread, so the guard has nothing to ask about.
  await click(await channelRow('#design'));
  assert.equal(dialog(), null);
  assert.deepEqual(refusals, []);
  await ui.waitFor(() => {
    assert.match(composer().placeholder, /#design/);
  });
  assert.equal(composer().value, '');
  await click(await channelRow('#general'));
  await ui.waitFor(() => {
    assert.equal(composer().value, 'half a thought');
  });
  await leave(store);
  assert.equal(dialog(), null);
  assert.deepEqual(store.getSnapshot().location, { kind: 'files' });
  await leave(store, IN_CHAT);
  await ui.waitFor(() => assert.equal(composer().value, 'half a thought'));
});

test('a draft is dropped when its channel stops being listed', async () => {
  let listed = true;
  const { store } = await setup({
    override: (base) => withDesign(base, () => listed),
    includeHousehold: true,
  });
  // The team's channels are re-listed when its conversation is mounted, so
  // leaving the team and coming back is what re-reads them.
  const relist = async (): Promise<void> => {
    await ui.act(async () => {
      store.navigate({ kind: 'chat', ref: 'team:household' }, { force: true });
      await Promise.resolve();
    });
    await ui.act(async () => {
      store.navigate(IN_CHAT, { force: true });
      await Promise.resolve();
    });
  };
  await click(await channelRow('#design'));
  await ui.waitFor(() => {
    assert.match(composer().placeholder, /#design/);
  });
  write('for the drawings');
  await click(await channelRow('#general'));
  await ui.waitFor(() => {
    assert.match(composer().placeholder, /#general/);
  });
  listed = false;
  await relist();
  // Only the general channel's row remains under the team's heading. The
  // column lists a second team as well, so the rows read are Engineering's.
  await ui.waitFor(() => {
    assert.deepEqual(engineeringChannels(), ['#general']);
  });
  listed = true;
  await relist();
  await click(await channelRow('#design'));
  await ui.waitFor(() => {
    assert.match(composer().placeholder, /#design/);
  });
  // The channel came back; the draft of the channel that had gone did not.
  assert.equal(composer().value, '');
});

test('local intent persistence survives navigation and retains the next draft', async () => {
  let finishSave!: () => void;
  let preparations = 0;
  let attempts = 0;
  const { store, refusals } = await setup({
    override: (base) => ({
      ...base,
      chatLocal: async (action) => {
        if (action.action === 'save-intent')
          await new Promise<void>((resolve) => {
            finishSave = resolve;
          });
        return base.chatLocal(action);
      },
      chat: async (storeId, action, view) => {
        if (action.action === 'submit-message') preparations++;
        if (action.action === 'attempt') attempts++;
        return base.chat(storeId, action, view);
      },
    }),
  });
  write('keep this until saved');
  await click(ui.screen.getByRole('button', { name: 'Send' }));
  const sending = await ui.waitFor(() => {
    const row = document.querySelector<HTMLElement>('.chat-outgoing.sending');
    assert.ok(row);
    return row;
  });
  assert.ok(ui.within(sending).getByText('keep this until saved'));
  assert.equal(ui.within(sending).queryByText('Sending…'), null);
  assert.equal(ui.within(sending).queryByText('Details'), null);
  assert.equal(composer().value, '');
  assert.equal(composer().disabled, false);
  write('the next draft');
  assert.equal(
    ui.screen.getByRole<HTMLButtonElement>('button', { name: 'Send' }).disabled,
    true,
  );
  await leave(store);
  assert.equal(dialog(), null);
  assert.deepEqual(store.getSnapshot().location, { kind: 'files' });
  assert.deepEqual(refusals, []);
  assert.equal(
    preparations,
    0,
    'network preparation must follow local persistence',
  );
  await ui.act(async () => {
    finishSave();
  });
  await ui.waitFor(() => assert.equal(preparations, 1));
  assert.deepEqual(store.getSnapshot().location, { kind: 'files' });
  await leave(store, IN_CHAT);
  await ui.waitFor(() => assert.equal(composer().value, 'the next draft'));
  await ui.screen.findByText('keep this until saved', {
    selector: '.chat-message p',
  });
  await ui.waitFor(() =>
    assert.equal(
      ui.screen.getByRole<HTMLButtonElement>('button', { name: 'Send' })
        .disabled,
      false,
    ),
  );
  assert.equal(preparations, 1);
  assert.equal(attempts, 0);
  assert.equal(document.querySelectorAll('.chat-message p').length, 2);
});

test('a lost submit reply restores the same saved submission after leaving chat', async () => {
  const submissions: string[] = [];
  let lost = true;
  let attempts = 0;
  const { store } = await setup({
    override: (base) => ({
      ...base,
      chat: async (storeId, action, view) => {
        if (
          action.action === 'submit-message' ||
          action.action === 'prepare-message'
        ) {
          submissions.push(action.submission);
          const reply = await base.chat(storeId, action, view);
          if (lost) {
            lost = false;
            throw {
              code: 'ambiguous',
              message: 'Preparation reply lost',
              ambiguous: true,
              retryable: false,
              fatal: false,
            };
          }
          return reply;
        }
        if (action.action === 'attempt') attempts++;
        return base.chat(storeId, action, view);
      },
    }),
  });
  write('durable before delivery');
  await click(ui.screen.getByRole('button', { name: 'Send' }));
  await ui.screen.findByRole('button', { name: 'Check again' });
  const submission = document
    .querySelector('.chat-outgoing')
    ?.getAttribute('data-submission');
  assert.ok(submission);
  assert.equal(composer().value, '');
  write('a later thought');
  await leave(store);
  assert.equal(dialog(), null);
  assert.deepEqual(store.getSnapshot().location, { kind: 'files' });
  await ui.waitFor(() => assert.equal(submissions.length, 2), {
    timeout: 5000,
  });
  assert.deepEqual(store.getSnapshot().location, { kind: 'files' });
  await leave(store, IN_CHAT);
  await ui.waitFor(() => assert.equal(composer().value, 'a later thought'));
  await ui.screen.findByText('durable before delivery', {
    selector: '.chat-message p',
  });
  assert.equal(
    ui.screen.getAllByText('durable before delivery', {
      selector: '.chat-message p',
    }).length,
    1,
  );
  assert.equal(submissions[0], submission);
  assert.equal(submissions.length, 2);
  assert.equal(submissions[0], submissions[1]);
  assert.equal(attempts, 0);
});

test('a message being sent is neither prompted about nor refused', async () => {
  const { store, refusals } = await setup({
    override: (base) => ({
      ...base,
      chat: async (storeId, action, view) => {
        // The preparation the agent has been given never answers, so the send
        // is still in flight when the move is asked for.
        if (action.action === 'submit-message')
          return new Promise<ChatReply>(() => {});
        return base.chat(storeId, action, view);
      },
    }),
  });
  write('on its way');
  await ui.act(async () => {
    ui.fireEvent.keyDown(composer(), { key: 'Enter' });
    await Promise.resolve();
  });
  await ui.waitFor(() =>
    assert.ok(document.querySelector('.chat-outgoing.sending')),
  );
  assert.equal(ui.screen.queryByText('Sending…'), null);
  const submission = document
    .querySelector('.chat-outgoing')
    ?.getAttribute('data-submission');
  assert.ok(submission);
  assert.equal(composer().disabled, false);
  assert.equal(composer().value, '');
  write('another thought');
  await leave(store);
  // Durable work is recovered in "Needs attention", so the move is allowed.
  assert.equal(dialog(), null);
  assert.deepEqual(refusals, []);
  assert.deepEqual(store.getSnapshot().location, { kind: 'files' });
  await leave(store, IN_CHAT);
  await ui.waitFor(() =>
    assert.ok(document.querySelector('.chat-outgoing.sending')),
  );
  assert.equal(ui.screen.queryByText('Sending…'), null);
  assert.equal(
    document.querySelector('.chat-outgoing')?.getAttribute('data-submission'),
    submission,
  );
  await ui.screen.findByText('on its way', { selector: '.chat-message p' });
  assert.equal(composer().value, 'another thought');
  assert.equal(
    ui.screen.getByRole<HTMLButtonElement>('button', { name: 'Send' }).disabled,
    true,
  );
});

test('an untouched New chat sheet does not prompt', async () => {
  const { store, refusals } = await setup();
  await openChannelSheet();
  await leave(store);
  assert.equal(dialog(), null);
  assert.deepEqual(refusals, []);
  assert.deepEqual(store.getSnapshot().location, { kind: 'files' });
});

test('a half-finished new channel names itself and Discard makes the move', async () => {
  const { store } = await setup();
  const name = await openChannelSheet();
  ui.fireEvent.change(name, { target: { value: 'Drawings' } });
  // Leaving chat is where the same draft is finally asked about.
  await leave(store);
  const panel = dialog();
  assert.ok(panel, 'the confirmation is up');
  assert.equal(
    panel.querySelector('.hd h2')?.textContent,
    'Discard the new channel?',
  );
  assert.equal(
    panel.querySelector('.sb p')?.textContent,
    '#drawings has not been created and will be lost.',
  );
  // Nothing has moved while the question is open.
  assert.deepEqual(store.getSnapshot().location, IN_CHAT);
  await click(dialogButton('Discard'));
  assert.equal(dialog(), null);
  assert.deepEqual(store.getSnapshot().location, { kind: 'files' });
});

test('channel creation completes after navigation without redirecting back', async () => {
  let finish!: () => void;
  let preparations = 0;
  let attempts = 0;
  const { store, refusals } = await setup({
    override: (base) => ({
      ...base,
      chat: async (storeId, action, view) => {
        if (action.action === 'prepare-channel') {
          preparations++;
          await new Promise<void>((resolve) => {
            finish = resolve;
          });
        }
        if (action.action === 'attempt') attempts++;
        return base.chat(storeId, action, view);
      },
    }),
  });
  const name = await openChannelSheet();
  ui.fireEvent.change(name, { target: { value: 'drawings' } });
  await click(ui.screen.getByRole('button', { name: 'Create channel' }));
  await ui.screen.findByRole('button', { name: 'Creating…' });
  await ui.waitFor(() => assert.ok(finish));
  await leave(store);
  assert.equal(dialog(), null);
  assert.deepEqual(refusals, []);
  assert.deepEqual(store.getSnapshot().location, { kind: 'files' });
  await ui.act(async () => {
    finish();
  });
  await ui.waitFor(() => assert.equal(attempts, 1));
  assert.deepEqual(store.getSnapshot().location, { kind: 'files' });
  await leave(store, IN_CHAT);
  await channelRow('#drawings');
  assert.deepEqual(store.getSnapshot().location, IN_CHAT);
  assert.equal(ui.screen.queryByRole('dialog'), null);
  assert.equal(preparations, 1);
  assert.equal(attempts, 1);
});

test('the tab’s own resolution of a chat location is not put to the guards', async () => {
  const { store } = await setup({
    start: { kind: 'chat' },
    guard: {
      verdict: 'prompt',
      title: 'Discard unsaved changes?',
      body: 'Your draft has not been saved.',
      confirm: 'Discard changes',
    },
  });
  // The tab resolved `{kind:'chat'}` into a team and a channel, and landed on
  // the conversation rather than on a question about it.
  const here = store.getSnapshot().location;
  assert.equal(here.kind, 'chat');
  assert.equal(here.kind === 'chat' ? here.ref : undefined, 'team:eng');
  assert.equal(dialog(), null);
});

test('a new channel resumes its name after a rail tab switch', async () => {
  const { store } = await setup();
  const name = await openChannelSheet();
  ui.fireEvent.change(name, { target: { value: 'Design notes' } });
  await ui.act(async () => {
    store.navigateTab('files');
  });
  assert.equal(dialog(), null);
  await ui.act(async () => {
    store.navigateTab('chat');
  });
  const restored = await ui.screen.findByLabelText('Channel name');
  assert.equal((restored as HTMLInputElement).value, 'design notes');
});
