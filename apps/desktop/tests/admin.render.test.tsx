import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactNode } from 'react';
import { installDom } from './lib/dom-harness';
import { workflowScope } from './lib/workflow-scope';
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

/** The sheet portals into the overlay root, so every panel needs a provider. */
async function overlay(children: ReactNode) {
  const { OverlayProvider } = await import('../kit/overlay-primitives');
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  return createElement(OverlayProvider, {
    backgroundRef: { current: null },
    portalRoot,
    children: await workflowScope(children),
  });
}

const presentation = {
  title: 'Manage via web',
  subtitle: 'ada on Example',
  onClose: () => {},
};

test('admin acknowledgments cannot return bearer URLs to the main webview', async () => {
  const { decodeCommandAck } = await import('../src/bridge');
  assert.deepEqual(decodeCommandAck({ ok: true }), { ok: true });
  assert.throws(() => decodeCommandAck({ ok: true, url: 'secret' }));
});
test('admin configuration and opening are distinct explicit actions and PIN is cleared', async () => {
  const { AdminPanel } =
    await import('../src/features/settings/account/admin-panel');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const calls: unknown[][] = [];
  const bridge = {
    ...mockBridge(),
    configureWebAdmin: async (...args: [string, string, string]) => {
      calls.push(args);
      return { ok: true as const };
    },
    openWebAdmin: async (...args: [string, string, string | null]) => {
      calls.push(args);
      throw new Error('Host administration unsupported');
    },
  };
  const r = ui.render(
    await overlay(
      createElement(AdminPanel, {
        bridge,
        profile: 'local',
        account: 'work',
        presentation,
      }),
    ),
  );
  ui.fireEvent.change(r.getByLabelText('Admin panel address'), {
    target: { value: 'https://admin.example/' },
  });
  ui.fireEvent.click(r.getByText('Save address'));
  await ui.waitFor(() => assert.ok(r.queryByText('Address saved.')));
  assert.deepEqual(calls, [['local', 'work', 'https://admin.example/']]);
  ui.fireEvent.change(
    r.getByLabelText('Security key PIN (enrolled keys only)'),
    {
      target: { value: '654321' },
    },
  );
  ui.fireEvent.click(r.getByText('Open admin panel'));
  await ui.waitFor(() => assert.ok(r.queryByRole('alert')));
  assert.deepEqual(calls[1], ['local', 'work', '654321']);
  assert.equal(
    (
      r.getByLabelText(
        'Security key PIN (enrolled keys only)',
      ) as HTMLInputElement
    ).value,
    '',
  );
  r.rerender(
    await overlay(
      createElement(AdminPanel, {
        bridge,
        profile: 'local',
        account: 'other',
        presentation,
      }),
    ),
  );
  await ui.waitFor(() => assert.ok(r.queryByRole('alert') === null));
});
