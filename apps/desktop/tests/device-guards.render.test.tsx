/**
 * What the device, key and passphrase sheets answer when the reader tries to
 * leave the page under them: nothing while they are pristine, a refusal while
 * a phrase is on screen or a write is out, and a question about what has only
 * been typed. The two canonicalizing redirects are here too, because they are
 * the moves no sheet may answer for.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactNode } from 'react';
import { createServer, type ViteDevServer } from 'vite';

import type { Bridge } from '../src/bridge';
import type { GuardVerdict, Location, LocationStore } from '../src/location';
import type { AccountStore, AgentSnapshot, StoreRef } from '../src/model';
import { installDom } from './lib/dom-harness';

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
test.after(async () => vite.close());

/** A call the sheet is still waiting on when the guard is asked. */
function never<T>(): Promise<T> {
  return new Promise<T>(() => {});
}

/** The move every test asks about: another page entirely. */
const AWAY: Location = { kind: 'files' };

async function harness() {
  const sheets = (await vite.ssrLoadModule(
    '/src/screens/device-sheets.tsx',
  )) as typeof import('../src/screens/device-sheets');
  const locations = (await vite.ssrLoadModule(
    '/src/location.ts',
  )) as typeof import('../src/location');
  const guards = (await vite.ssrLoadModule(
    '/src/navigation-guard.tsx',
  )) as typeof import('../src/navigation-guard');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const { ToastController, ToastProvider } = (await vite.ssrLoadModule(
    '/kit/toasts.tsx',
  )) as typeof import('../kit/toasts');
  const { OverlayProvider } = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives');
  const { WorkflowProvider } = (await vite.ssrLoadModule(
    '/src/workflow-context.tsx',
  )) as typeof import('../src/workflow-context');
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  const account = FIXTURE.stores.find(
    (store): store is AccountStore =>
      store.kind === 'account' && store.id === 'acct:personal',
  );
  assert.ok(account, 'the fixture holds the Personal account');

  /**
   * Renders one sheet under a store of its own, with the prompter the shell
   * installs replaced by `answer` so a prompted move settles without a dialog.
   */
  const mount = (
    node: ReactNode,
    answer: boolean | null = null,
  ): {
    store: LocationStore;
    refusals: string[];
    asked: GuardVerdict[];
    rendered: ReturnType<typeof ui.render>;
  } => {
    const store = new locations.LocationStore();
    const refusals: string[] = [];
    const asked: GuardVerdict[] = [];
    store.setRefusalHandler((reason) => refusals.push(reason));
    if (answer !== null)
      store.setPrompter(async (verdict) => {
        asked.push(verdict);
        return answer;
      });
    const rendered = ui.render(
      createElement(OverlayProvider, {
        backgroundRef: { current: null },
        portalRoot,
        children: createElement(ToastProvider, {
          controller: new ToastController(),
          children: createElement(guards.NavigationGuardProvider, {
            store,
            children: createElement(WorkflowProvider, {
              snapshot: FIXTURE,
              children: node,
            }),
          }),
        }),
      }),
    );
    return { store, refusals, asked, rendered };
  };

  return {
    sheets,
    account,
    mount,
    bridge: (overrides: Partial<Bridge> = {}): Bridge => ({
      ...mockBridge(FIXTURE),
      ...overrides,
    }),
    /** What the guards say about leaving, without moving anything. */
    verdict: (store: LocationStore): GuardVerdict =>
      store.navigationVerdict({ kind: 'navigate', location: AWAY }),
    /** Makes the move and lets a prompt settle. */
    leave: async (store: LocationStore): Promise<void> => {
      await ui.act(async () => {
        store.navigate(AWAY);
        await Promise.resolve();
      });
    },
  };
}

/* ---------------------------------------------------------- paper key -- */

test('paper key modal allows navigation when no phrase is displayed', async () => {
  const h = await harness();
  const { store } = h.mount(
    createElement(h.sheets.PhraseSheet, {
      bridge: h.bridge(),
      profile: 'personal',
      accountAlias: 'personal',
      onClose: () => {},
      onDone: async () => {},
      onError: () => {},
    }),
  );
  assert.equal(h.verdict(store), null);
});

test('revealed paper key blocks navigation and displays guidance message', async () => {
  const h = await harness();
  const closed: boolean[] = [];
  const { store, refusals } = h.mount(
    createElement(h.sheets.PhraseSheet, {
      bridge: h.bridge(),
      profile: 'personal',
      accountAlias: 'personal',
      seedPhrase: 'alpha bravo charlie delta',
      onClose: () => closed.push(true),
      onDone: async () => {},
      onError: () => {},
    }),
  );
  assert.deepEqual(h.verdict(store), {
    verdict: 'refuse',
    reason: 'Save or dismiss the paper key first.',
  });
  await h.leave(store);
  // A refusal moves nothing, raises no question, and leaves the sheet open.
  assert.deepEqual(store.getSnapshot().location, { kind: 'all' });
  assert.deepEqual(refusals, ['Save or dismiss the paper key first.']);
  assert.deepEqual(closed, []);
});

/* ------------------------------------------------------------ pairing -- */

function pairSheet(
  h: Awaited<ReturnType<typeof harness>>,
  mode: 'offer' | 'accept',
  bridge: Bridge,
  onClose: () => void = () => {},
): ReactNode {
  return createElement(h.sheets.PairSheet, {
    bridge,
    store: h.account,
    initialMode: mode,
    onCopy: () => {},
    onClose,
    onDone: async () => {},
    onError: () => {},
  });
}

test('an untouched accept form answers nothing', async () => {
  const h = await harness();
  const { store, rendered } = h.mount(pairSheet(h, 'accept', h.bridge()));
  assert.equal(h.verdict(store), null);
  assert.match(
    rendered.baseElement.textContent ?? '',
    /start or resume a pairing/,
  );
  assert.doesNotMatch(
    rendered.baseElement.textContent ?? '',
    /shown once|shows a phrase once/,
  );
});

for (const action of ['Start', 'Resume offer']) {
  test(`${action} explains that a pending pairing phrase can be shown again`, async () => {
    const h = await harness();
    const offer = {
      accountAlias: h.account.account,
      phrase: 'alpha bravo charlie delta',
    };
    const { rendered } = h.mount(
      pairSheet(
        h,
        'offer',
        h.bridge({
          startDevicePairing: async () => offer,
          resumeDevicePairingOffer: async () => offer,
        }),
      ),
    );
    ui.fireEvent.click(rendered.getByRole('button', { name: action }));
    await ui.waitFor(() => assert.ok(rendered.getByText(offer.phrase)));
    assert.match(
      rendered.baseElement.textContent ?? '',
      /Use Resume offer to show it again while pairing is pending/,
    );
    assert.doesNotMatch(
      rendered.baseElement.textContent ?? '',
      /shown once|shows a phrase once/,
    );
    if (action === 'Resume offer')
      assert.ok(
        rendered.getByText('The phrase below is the one already issued.'),
      );
  });
}

test('a pairing the agent is holding refuses the move', async () => {
  const h = await harness();
  const { store, refusals, rendered } = h.mount(
    pairSheet(h, 'offer', h.bridge({ startDevicePairing: () => never() })),
  );
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Start' }));
    await Promise.resolve();
  });
  assert.deepEqual(h.verdict(store), {
    verdict: 'refuse',
    reason: 'Finish or cancel the pairing first.',
  });
  await h.leave(store);
  assert.deepEqual(store.getSnapshot().location, { kind: 'all' });
  assert.deepEqual(refusals, ['Finish or cancel the pairing first.']);
});

test('a typed pairing phrase is asked about, and confirming discards it', async () => {
  const h = await harness();
  const closed: boolean[] = [];
  const { store, asked, rendered } = h.mount(
    pairSheet(h, 'accept', h.bridge(), () => closed.push(true)),
    true,
  );
  const field = rendered.getByLabelText('Pairing phrase');
  await ui.act(async () => {
    ui.fireEvent.change(field, { target: { value: 'seven eight nine' } });
  });
  const question = h.verdict(store);
  assert.equal(question?.verdict, 'prompt');
  assert.equal(question.title, 'Discard pairing phrase?');
  assert.equal(
    question.body,
    'The pairing phrase typed here has not been submitted.',
  );
  assert.equal(question.confirm, 'Discard');

  await h.leave(store);
  assert.equal(asked.length, 1);
  assert.deepEqual(store.getSnapshot().location, AWAY);
  // Confirming runs the sheet's own discard: the field is cleared and the
  // sheet is closed before the move it asked about is made.
  assert.deepEqual(closed, [true]);
  assert.equal((field as HTMLInputElement).value, '');
});

test('canceling the question keeps the phrase and the page', async () => {
  const h = await harness();
  const closed: boolean[] = [];
  const { store, asked, rendered } = h.mount(
    pairSheet(h, 'accept', h.bridge(), () => closed.push(true)),
    false,
  );
  const field = rendered.getByLabelText('Pairing phrase');
  await ui.act(async () => {
    ui.fireEvent.change(field, { target: { value: 'seven eight nine' } });
  });
  await h.leave(store);
  assert.equal(asked.length, 1);
  assert.deepEqual(store.getSnapshot().location, { kind: 'all' });
  assert.deepEqual(closed, []);
  assert.equal((field as HTMLInputElement).value, 'seven eight nine');
});

/* --------------------------------------------------------- passphrase -- */

test('a typed passphrase is asked about and an outstanding change is not', async () => {
  const h = await harness();
  const bridge = h.bridge({ changeAccountPassphrase: () => never() });
  const { store, rendered } = h.mount(
    createElement(h.sheets.PassphraseSheet, {
      bridge,
      store: h.account,
      initialMode: 'change',
      onClose: () => {},
      onDone: () => {},
      onError: () => {},
    }),
  );
  assert.equal(h.verdict(store), null);

  await ui.act(async () => {
    ui.fireEvent.change(rendered.getByLabelText('Passphrase'), {
      target: { value: 'correct horse' },
    });
    ui.fireEvent.change(rendered.getByLabelText('Confirm'), {
      target: { value: 'correct horse' },
    });
  });
  const typed = h.verdict(store);
  assert.equal(typed?.verdict, 'prompt');
  assert.equal(typed.title, 'Discard passphrase?');
  assert.equal(typed.body, 'The passphrase typed here has not been submitted.');
  assert.equal(typed.confirm, 'Discard');

  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'Change passphrase' }),
    );
    await Promise.resolve();
  });
  assert.deepEqual(h.verdict(store), {
    verdict: 'refuse',
    reason: 'Wait for the passphrase change to finish.',
  });
});

/* ------------------------------------------------------------- revoke -- */

test('a revocation already sent refuses the move', async () => {
  const h = await harness();
  const { store, rendered } = h.mount(
    createElement(h.sheets.RevokeSheet, {
      bridge: h.bridge({ runYubi: () => never() }),
      store: h.account,
      alias: 'work-key',
      onClose: () => {},
      onDone: async () => {},
      onError: () => {},
    }),
  );
  const confirmation = rendered.getByPlaceholderText('type work-key');
  await ui.act(async () => {
    ui.fireEvent.change(confirmation, { target: { value: 'work-key' } });
  });
  // The typed confirmation alone is not worth a question.
  assert.equal(h.verdict(store), null);
  await ui.act(async () => {
    ui.fireEvent.click(
      rendered.getByRole('button', { name: 'Revoke work-key' }),
    );
    await Promise.resolve();
  });
  assert.deepEqual(h.verdict(store), {
    verdict: 'refuse',
    reason: 'Wait for the revocation to finish.',
  });
});

/* ---------------------------------------------------------- redirects -- */

test('the two default routes are canonicalized without asking the guards', async () => {
  const h = await harness();
  const { DevicesScreen } = (await vite.ssrLoadModule(
    '/src/screens/devices-screen.tsx',
  )) as typeof import('../src/screens/devices-screen');
  const { PeopleScreen } = (await vite.ssrLoadModule(
    '/src/screens/people-screen.tsx',
  )) as typeof import('../src/screens/people-screen');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const snapshot: AgentSnapshot = FIXTURE;

  for (const [screen, kind] of [
    [DevicesScreen, 'devices'],
    [PeopleScreen, 'people'],
  ] as const) {
    const moves: { location: Location; force: boolean }[] = [];
    h.mount(
      createElement(screen as typeof DevicesScreen, {
        snapshot,
        bridge: h.bridge(),
        location: { kind } as Extract<Location, { kind: 'devices' }>,
        scene: kind,
        onNavigate: (location, options) =>
          moves.push({ location, force: Boolean(options?.force) }),
        onRefresh: async () => {},
        onRefreshSnapshot: async () => snapshot,
        onError: () => {},
        onMutationError: async () => {},
      }),
    );
    await ui.act(async () => {
      await Promise.resolve();
    });
    const first = moves[0];
    assert.ok(first, `${kind} canonicalizes its default route`);
    assert.equal(first.force, true);
    assert.equal(
      (first.location as { store?: StoreRef }).store,
      'acct:personal',
    );
    ui.cleanup();
  }
});
