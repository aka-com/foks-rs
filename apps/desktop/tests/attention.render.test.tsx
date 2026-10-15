/**
 * The header's attention bell in the mounted shell: it lists what the rail's
 * signals stand for, one entity at a time, links each to its page, carries
 * the setup steps still open, and lists the session's toast messages.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { installDom } from './lib/dom-harness';
import type { Bridge } from '../src/bridge';

installDom({
  url: 'http://localhost/?state=files',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
  act: true,
});
let ui: typeof import('@testing-library/react');
test.before(async () => {
  ui = await import('@testing-library/react');
});
test.afterEach(() => {
  ui.cleanup();
  window.localStorage.clear();
  Reflect.deleteProperty(document, 'hidden');
});

const UNAVAILABLE = {
  locked: false,
  available: false,
  mechanism: 'none' as const,
  unavailableReason: 'polkit is not installed',
};

function panel(): HTMLElement {
  const found = document.getElementById('attention-centre');
  assert.ok(found, 'the attention popover is open');
  return found;
}

async function openBell(): Promise<HTMLElement> {
  const bell = document.querySelector<HTMLButtonElement>('.attention-trigger');
  assert.ok(bell);
  if (bell.getAttribute('aria-expanded') !== 'true')
    await ui.act(async () => {
      ui.fireEvent.click(bell);
    });
  return panel();
}

test('the bell lists each signal, links it to its page, and keeps the setup steps and recent messages', async () => {
  // The shell reads device metadata only while the window is visible.
  Object.defineProperty(document, 'hidden', {
    configurable: true,
    value: false,
  });
  const { App } = await import('../src/app-root');
  const { FIXTURE } = await import('../src/fixture');
  const { mockBridge } = await import('../src/mock-bridge');
  const { deviceAlertRegistry } = await import('../src/data/device-alert');
  const base = mockBridge(FIXTURE);
  const bridge: Bridge = {
    ...base,
    appLockState: async () => UNAVAILABLE,
    lockApp: async () => UNAVAILABLE,
  };
  ui.render(createElement(App, { bridge }));
  // The shell preloads each account's keys; the work account has one device
  // and no recovery phrase.
  await ui.waitFor(() => {
    const facts = deviceAlertRegistry(bridge).getSnapshot();
    assert.equal(facts.phraseLists.get('acct:work')?.length, 0);
    assert.deepEqual(facts.keyCounts.get('acct:work'), { devices: 1 });
  });
  await ui.waitFor(() => {
    const bell = document.querySelector('.attention-trigger');
    assert.match(bell?.getAttribute('aria-label') ?? '', /need attention$/);
  });

  let open = await openBell();
  const rows = [...open.querySelectorAll('.attention-row')].map(
    (row) => row.textContent ?? '',
  );
  assert.ok(rows.some((text) => /Server not verified/.test(text)));
  assert.ok(
    rows.some((text) => /No recovery phrase.*vitalik on Acme/.test(text)),
  );
  assert.ok(
    rows.some((text) =>
      /To do: Add a second device or a hardware key.*vitalik on Acme has one key/.test(
        text,
      ),
    ),
  );
  assert.ok(
    rows.some((text) =>
      /Not available: App lock.*polkit is not installed/.test(text),
    ),
  );
  assert.equal(
    open.querySelector('.attention-recent'),
    null,
    'no message has been shown yet',
  );

  // An item is a link to the page that resolves it.
  const missing = [
    ...open.querySelectorAll<HTMLButtonElement>('button.attention-row'),
  ].find((row) => /No recovery phrase/.test(row.textContent ?? ''));
  assert.ok(missing);
  await ui.act(async () => {
    ui.fireEvent.click(missing);
  });
  assert.equal(document.getElementById('attention-centre'), null);
  assert.match(window.location.search, /state=devices/);
  assert.match(decodeURIComponent(window.location.search), /acct:work/);

  // A toast's message stays in Recent after the toast has gone.
  await ui.act(async () => {
    ui.fireEvent.click(document.querySelector('.rail-foot .who')!);
  });
  await ui.act(async () => {
    ui.fireEvent.click(ui.screen.getByRole('menuitem', { name: 'Lock' }));
  });
  await ui.waitFor(() =>
    assert.ok(
      document.body.textContent?.includes(
        'Application lock is not available on this system.',
      ),
    ),
  );
  open = await openBell();
  assert.deepEqual(
    [...open.querySelectorAll('.attention-recent .t')].map(
      (node) => node.textContent,
    ),
    ['Application lock is not available on this system.'],
  );

  // Hiding the checklist is a stored preference.
  await ui.act(async () => {
    ui.fireEvent.click(ui.within(open).getByRole('button', { name: 'Hide' }));
  });
  assert.equal(open.textContent?.includes('Get set up'), false);
  assert.equal(
    window.localStorage.getItem('onboarding.dismissed.setup-checklist'),
    'true',
  );
});

test('the recovery-phrase step opens Create recovery phrase on the account that lacks one', async () => {
  Object.defineProperty(document, 'hidden', {
    configurable: true,
    value: false,
  });
  const { App } = await import('../src/app-root');
  const { FIXTURE } = await import('../src/fixture');
  const { mockBridge } = await import('../src/mock-bridge');
  ui.render(createElement(App, { bridge: mockBridge(FIXTURE) }));
  const step = await ui.waitFor(async () => {
    const found = [
      ...(await openBell()).querySelectorAll<HTMLButtonElement>(
        'button.attention-row.step',
      ),
    ].find((row) => /Create a recovery phrase/.test(row.textContent ?? ''));
    assert.ok(found);
    return found;
  });
  await ui.act(async () => {
    ui.fireEvent.click(step);
  });
  await ui.screen.findByRole('dialog', { name: 'Create recovery phrase' });
  assert.match(decodeURIComponent(window.location.search), /acct:work/);
  assert.doesNotMatch(window.location.search, /open=/);
});

test('a lock that holds clears the recent messages', async () => {
  const { App } = await import('../src/app-root');
  const { FIXTURE } = await import('../src/fixture');
  const { mockBridge } = await import('../src/mock-bridge');
  const { ToastController } = await import('../kit/toasts');
  // The lock also unmounts the shell and its controller, so the controller
  // the shell connected is held here to read its history afterwards.
  const controllers = new Set<InstanceType<typeof ToastController>>();
  // Restored below, and only ever called with a controller as `this`.
  // eslint-disable-next-line @typescript-eslint/unbound-method
  const connect = ToastController.prototype.connect;
  ToastController.prototype.connect = function (listener) {
    controllers.add(this);
    return connect.call(this, listener);
  };
  try {
    const bridge: Bridge = {
      ...mockBridge(FIXTURE),
      lockApp: async () => ({
        locked: true,
        available: true,
        mechanism: 'password',
      }),
    };
    ui.render(createElement(App, { bridge }));
    await ui.waitFor(() =>
      assert.ok(document.querySelector('.attention-trigger')),
    );
    assert.equal(controllers.size, 1);
    const [toasts] = controllers;
    await ui.act(async () => {
      toasts.show('Saved');
    });
    const open = await openBell();
    assert.deepEqual(
      [...open.querySelectorAll('.attention-recent .t')].map(
        (node) => node.textContent,
      ),
      ['Saved'],
    );
    await ui.act(async () => {
      ui.fireEvent.keyDown(document, { key: 'Escape' });
    });
    await ui.act(async () => {
      ui.fireEvent.click(document.querySelector('.rail-foot .who')!);
    });
    await ui.act(async () => {
      ui.fireEvent.click(ui.screen.getByRole('menuitem', { name: 'Lock' }));
    });
    await ui.screen.findByRole('button', { name: 'Unlock' });
    assert.deepEqual(toasts.getHistory(), []);
  } finally {
    ToastController.prototype.connect = connect;
  }
});

test('an interrupted membership change is read once per profile and links to its team', async () => {
  const { App } = await import('../src/app-root');
  const { FIXTURE } = await import('../src/fixture');
  const { mockBridge } = await import('../src/mock-bridge');
  const reads = new Map<string, number>();
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    listPendingOperations: async (profile) => {
      reads.set(profile, (reads.get(profile) ?? 0) + 1);
      return profile === 'acme'
        ? [
            { kind: 'team-member-edit', alias: 'engineering' },
            // Another kind of journalled work is not a membership change.
            { kind: 'team-rekey', alias: 'engineering' },
          ]
        : [];
    },
  };
  ui.render(createElement(App, { bridge }));
  const row = await ui.waitFor(async () => {
    const found = [
      ...(await openBell()).querySelectorAll<HTMLButtonElement>(
        'button.attention-row',
      ),
    ].find((entry) =>
      /1 incomplete membership change in Engineering/.test(
        entry.textContent ?? '',
      ),
    );
    assert.ok(found);
    return found;
  });
  assert.equal(reads.get('acme'), 1);
  await ui.act(async () => {
    ui.fireEvent.click(row);
  });
  assert.match(decodeURIComponent(window.location.search), /team:eng/);
});

test('a warning in Recent is marked by more than its colour', async () => {
  const { OverlayProvider } = await import('../kit/overlay-primitives');
  const { AttentionControl } = await import('../src/shell/attention-centre');
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  ui.render(
    createElement(OverlayProvider, {
      backgroundRef: { current: null },
      portalRoot,
      children: createElement(AttentionControl, {
        items: [],
        checklist: [],
        recent: [
          { id: 1, message: 'Saved', tone: 'info', shownAt: 0 },
          { id: 2, message: 'Sync stopped', tone: 'warning', shownAt: 0 },
          { id: 3, message: 'Restore failed', tone: 'warning', shownAt: 0 },
        ],
        onNavigate: () => undefined,
      }),
    }),
  );
  const open = await openBell();
  // Newest first; every warning carries the alert icon and spoken prefix.
  assert.deepEqual(
    [...open.querySelectorAll('.attention-recent li')].map((entry) => [
      entry.textContent?.replace(/\d\d:\d\d.*$/, ''),
      entry.querySelector('.ic') !== null,
    ]),
    [
      ['Warning: Restore failed', true],
      ['Warning: Sync stopped', true],
      ['Saved', false],
    ],
  );
});
