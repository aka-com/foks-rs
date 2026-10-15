import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { Bridge, PendingOperation } from '../src/bridge';
import { installDom } from './lib/dom-harness';
import { loadTestShell } from './lib/test-shell';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
});
let ui: typeof import('@testing-library/react');
test.before(async () => {
  ui = await import('@testing-library/react');
});
test.afterEach(() => {
  ui.cleanup();
  window.history.replaceState(null, '', '/');
});

for (const action of ['add', 'demote', 'promote'] as const) {
  const kind = action === 'add' ? 'team-member-addition' : 'team-member-edit';
  test(`exposes and completes ${action} ${kind} after an interrupted change without reopening settings`, async () => {
    const { GroupSettingsScreen } =
      await import('../src/screens/groups-screen');
    const TestShell = await loadTestShell();
    const { ToastProvider, ToastController } = await import('../kit/toasts');
    const { OverlayProvider } = await import('../kit/overlay-primitives');
    const { FIXTURE } = await import('../src/mock/fixture');
    const { mockBridge } = await import('../src/mock/mock-bridge');
    const addition = kind === 'team-member-addition';
    window.history.replaceState(null, '', `/?state=${action}`);
    const store = FIXTURE.stores.find((entry) => entry.id === 'team:eng');
    assert.ok(store?.kind === 'team');
    const portalRoot = document.getElementById('overlays');
    assert.ok(portalRoot);
    let pending: PendingOperation[] = [];
    let reads = 0;
    let reconciled = false;
    let resumed = false;
    const interrupted = async () => {
      pending = [
        {
          kind,
          alias: store.alias,
          ...(addition ? { target: 'jules.park' } : {}),
        },
      ];
      throw {
        code: 'ambiguous',
        message: 'Interrupted',
        retryable: false,
        ambiguous: true,
        fatal: false,
      };
    };
    const bridge: Bridge = {
      ...mockBridge(FIXTURE),
      listPendingOperations: async () => {
        reads++;
        return pending;
      },
      addGroupMember: interrupted,
      demoteGroupMember: interrupted,
      promoteGroupMember: interrupted,
      resumeGroupMemberAddition: async (request) => {
        assert.equal(request.storeId, store.id);
        assert.equal(request.username, 'jules.park');
        resumed = true;
        pending = [];
        return { applied: true };
      },
      resumeGroupMemberEdit: async (storeId) => {
        assert.equal(storeId, store.id);
        resumed = true;
        pending = [];
        return { applied: true };
      },
    };
    const rendered = ui.render(
      createElement(OverlayProvider, {
        backgroundRef: { current: null },
        portalRoot,
        children: createElement(ToastProvider, {
          controller: new ToastController(),
          children: createElement(TestShell, {
            snapshot: FIXTURE,
            bridge,
            children: createElement(GroupSettingsScreen, {
              snapshot: FIXTURE,
              bridge,
              location: {
                kind: 'group-settings',
                ref: store.id,
                tab: 'people',
              },
              onNavigate: () => {},
              onApplied: async () => {},
              onError: (error: unknown) => {
                throw error;
              },
              onMutationError: async () => {
                reconciled = true;
              },
            }),
          }),
        }),
      }),
    );
    await ui.act(async () => {});
    assert.equal(reads, 1);
    assert.equal(rendered.queryByText('Unfinished activity'), null);
    // The username field starts empty, so the addition names its member by
    // being typed rather than by arriving prefilled.
    if (addition)
      ui.fireEvent.change(rendered.getByLabelText('Username'), {
        target: { value: 'jules.park' },
      });
    await ui.act(async () => {
      ui.fireEvent.click(
        rendered.getByRole('button', {
          name: addition ? 'Add jules.park' : 'Change role',
        }),
      );
    });
    assert.ok(reconciled);
    // The journal is held for minutes, so the resume offer can only come
    // from the read the write's own outcome asked for, not from the
    // freshness window elapsing.
    assert.equal(reads, 2);
    // Close the failed action sheet before opening the activity sheet from
    // the updated summary band.
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Cancel' }));
    // The toast host's polite region is a status too.
    const band = rendered
      .getAllByRole('status')
      .find((status) => !status.closest('#toasts'));
    assert.ok(band);
    assert.equal(
      band.textContent,
      'Unfinished activity 1 incomplete membership change. It blocks other actions.Review',
    );
    await ui.act(async () => {
      ui.fireEvent.click(
        ui.within(band).getByRole('button', { name: 'Review' }),
      );
    });
    const resumeName = addition
      ? 'Resume adding jules.park'
      : 'Resume the role change';
    await ui.act(async () => {
      ui.fireEvent.click(rendered.getByRole('button', { name: resumeName }));
    });
    assert.ok(resumed);
    assert.equal(reads, 3);
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Close' }));
    assert.equal(rendered.queryByText('Unfinished activity'), null);
  });
}

test('a failed pending-operation query reports one error and renders no recovery banner', async () => {
  const { GroupSettingsScreen } = await import('../src/screens/groups-screen');
  const TestShell = await loadTestShell();
  const { ToastProvider, ToastController } = await import('../kit/toasts');
  const { OverlayProvider } = await import('../kit/overlay-primitives');
  const { FIXTURE } = await import('../src/mock/fixture');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const store = FIXTURE.stores.find((entry) => entry.id === 'team:eng');
  assert.ok(store?.kind === 'team');
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  const reported: unknown[] = [];
  const bridge: Bridge = {
    ...mockBridge(FIXTURE),
    listPendingOperations: async () => {
      throw {
        code: 'io',
        message: 'The pending operations could not be read.',
        retryable: true,
        ambiguous: false,
        fatal: false,
      };
    },
  };
  const rendered = ui.render(
    createElement(OverlayProvider, {
      backgroundRef: { current: null },
      portalRoot,
      children: createElement(ToastProvider, {
        controller: new ToastController(),
        children: createElement(TestShell, {
          snapshot: FIXTURE,
          bridge,
          children: createElement(GroupSettingsScreen, {
            snapshot: FIXTURE,
            bridge,
            location: { kind: 'group-settings', ref: store.id, tab: 'people' },
            onNavigate: () => {},
            onApplied: async () => {},
            onError: (error: unknown) => {
              reported.push(error);
            },
            onMutationError: async () => {},
          }),
        }),
      }),
    }),
  );
  await ui.act(async () => {});
  await ui.waitFor(() => assert.equal(reported.length, 1));
  assert.match(
    (reported[0] as { message: string }).message,
    /could not be read/,
  );
  assert.equal(rendered.queryByText('Unfinished activity'), null);
});
