import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactNode } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import { installDom } from './lib/dom-harness';
import { workflowScope } from './lib/workflow-scope';
import { decodeBotReply } from '../src/bot-contract';
import type { BotEnrollment, BotAction } from '../src/bot-contract';
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

/** The sheet portals into the overlay root, so every panel needs a provider. */
async function overlay(children: ReactNode) {
  const { OverlayProvider } = (await vite.ssrLoadModule(
    '/kit/overlay-primitives.tsx',
  )) as typeof import('../kit/overlay-primitives');
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  return createElement(OverlayProvider, {
    backgroundRef: { current: null },
    portalRoot,
    children: await workflowScope(vite, children),
  });
}

const presentation = {
  title: 'Bot accounts',
  subtitle: 'ada on Example',
  onClose: () => {},
};
const progress: BotEnrollment = {
  operation_id: '1'.repeat(32),
  account_alias: 'work',
  name: 'abcde',
  device_id: '13' + '0'.repeat(64),
  role: 'OWNER',
  state: 'prepared',
  hardware_required: false,
  export_available: false,
};
test('bot DTOs reject secret fields and wrong handles', () => {
  assert.deepEqual(
    decodeBotReply({ rows: [progress], message: 'ready' }).rows,
    [progress],
  );
  for (const key of ['token', 'seed', 'pin'])
    assert.throws(() =>
      decodeBotReply({
        rows: [{ ...progress, [key]: 'secret' }],
        message: 'ready',
      }),
    );
  assert.throws(() =>
    decodeBotReply({
      rows: [{ ...progress, operation_id: 'bad' }],
      message: 'ready',
    }),
  );
});
test('hardware-needed enrollment stays passive until a PIN is explicitly supplied', async () => {
  const { BotPanel } = await vite.ssrLoadModule('/src/components/bot-panel.tsx') as typeof import('../src/components/bot-panel');
  const { mockBridge } = await vite.ssrLoadModule('/src/mock-bridge.ts') as typeof import('../src/mock-bridge');
  const actions: BotAction[] = [];
  let scans = 0;
  const bridge = {
    ...mockBridge(),
    listYubiCards: async () => { scans++; return []; },
    botAccount: async (_profile: string, _account: string, action: BotAction) => {
      actions.push(action);
      return { rows: [{ ...progress, hardware_required: true }], message: 'Hardware needed' };
    },
  };
  const r = ui.render(await overlay(createElement(BotPanel, {
    bridge, profile: 'local', account: 'work', presentation, onComplete() {},
  })));
  assert.equal(actions.length, 0);
  ui.fireEvent.click(r.getByText('Enroll bot'));
  await ui.waitFor(() => assert.ok(r.queryByText('Confirm')));
  assert.equal((r.getByText('Confirm') as HTMLButtonElement).disabled, true);
  assert.equal(scans, 0);
  assert.equal(actions.length, 1);
  ui.fireEvent.change(r.getByLabelText('Security key PIN (enrolled keys only)'), {
    target: { value: '654321' },
  });
  ui.fireEvent.click(r.getByText('Confirm'));
  await ui.waitFor(() => assert.equal(actions.length, 2));
  assert.equal(scans, 0);
});

test('explicit role, confirmation and unknown recovery never export or replay', async () => {
  const { BotPanel } = (await vite.ssrLoadModule(
    '/src/components/bot-panel.tsx',
  )) as typeof import('../src/components/bot-panel');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const actions: BotAction[] = [];
  const bridge = {
    ...mockBridge(),
    botAccount: async (_p: string, _a: string, action: BotAction) => {
      actions.push(action);
      return {
        rows: [
          {
            ...progress,
            state:
              action.action === 'prepare' ? 'prepared' : 'submission-unknown',
          },
        ],
        message: 'updated',
      };
    },
  };
  const r = ui.render(
    await overlay(
      createElement(BotPanel, {
        bridge,
        profile: 'local',
        account: 'work',
        presentation,
        onComplete: () => {},
      }),
    ),
  );
  ui.fireEvent.change(r.getByLabelText('Role'), {
    target: { value: 'owner' },
  });
  ui.fireEvent.change(
    r.getByLabelText('Security key PIN (enrolled keys only)'),
    {
      target: { value: '654321' },
    },
  );
  ui.fireEvent.click(r.getByText('Enroll bot'));
  await ui.waitFor(() => assert.ok(r.queryByText('Confirm')));
  assert.deepEqual(actions[0], {
    action: 'prepare',
    role: 'owner',
    pin: '654321',
  });
  assert.equal(
    (
      r.getByLabelText(
        'Security key PIN (enrolled keys only)',
      ) as HTMLInputElement
    ).value,
    '',
  );
  ui.fireEvent.click(r.getByText('Confirm'));
  await ui.waitFor(() => assert.ok(r.queryByText('Confirm') === null));
  assert.ok(r.queryByText('Export token') === null);
  ui.fireEvent.click(r.getByText('Check status'));
  await ui.waitFor(() => assert.equal(actions.length, 3));
  assert.equal(actions[2].action, 'status');
  r.rerender(
    await overlay(
      createElement(BotPanel, {
        bridge,
        profile: 'local',
        account: 'other',
        presentation,
        onComplete: () => {},
      }),
    ),
  );
  await ui.waitFor(() => assert.ok(r.queryByText('Check status') === null));
});

test('revocation accepts a security key PIN directly in the Revoke pane', async () => {
  const { BotPanel } = (await vite.ssrLoadModule(
    '/src/components/bot-panel.tsx',
  )) as typeof import('../src/components/bot-panel');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const actions: BotAction[] = [];
  const bridge = {
    ...mockBridge(),
    botAccount: async (
      _profile: string,
      _account: string,
      action: BotAction,
    ) => {
      actions.push(action);
      return { rows: [], message: 'Revoked' };
    },
  };
  const r = ui.render(
    await overlay(
      createElement(BotPanel, {
        bridge,
        profile: 'local',
        account: 'work',
        presentation,
        onComplete: () => {},
      }),
    ),
  );
  ui.fireEvent.click(r.getByRole('button', { name: 'Revoke' }));
  ui.fireEvent.change(r.getByLabelText('Credential ID'), {
    target: { value: progress.device_id },
  });
  ui.fireEvent.change(
    r.getByLabelText('Security key PIN (enrolled keys only)'),
    { target: { value: '654321' } },
  );
  ui.fireEvent.click(r.getAllByRole('button', { name: 'Revoke' }).at(-1)!);
  ui.fireEvent.click(r.getByRole('button', { name: 'Confirm revocation' }));
  await ui.waitFor(() => assert.equal(actions.length, 1));
  assert.deepEqual(actions[0], {
    action: 'revoke',
    device_id: progress.device_id,
    pin: '654321',
  });
  assert.equal(
    (
      r.getByLabelText(
        'Security key PIN (enrolled keys only)',
      ) as HTMLInputElement
    ).value,
    '',
  );
});
