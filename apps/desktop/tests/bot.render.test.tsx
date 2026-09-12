import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import { installDom } from './lib/dom-harness';
import { decodeBotReply } from '../src/bot-contract';
import type { BotEnrollment, BotAction } from '../src/bot-contract';
installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div>',
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
    createElement(BotPanel, {
      bridge,
      profile: 'local',
      account: 'work',
      onComplete: () => {},
    }),
  );
  ui.fireEvent.change(r.getByLabelText('Bot role'), {
    target: { value: 'owner' },
  });
  ui.fireEvent.change(r.getByLabelText('Security key PIN, if needed'), {
    target: { value: '654321' },
  });
  ui.fireEvent.click(r.getByText('Prepare bot enrollment'));
  await ui.waitFor(() => assert.ok(r.queryByText('Confirm bot enrollment')));
  assert.deepEqual(actions[0], {
    action: 'prepare',
    role: 'owner',
    pin: '654321',
  });
  assert.equal(
    (r.getByLabelText('Security key PIN, if needed') as HTMLInputElement).value,
    '',
  );
  ui.fireEvent.click(r.getByText('Confirm bot enrollment'));
  await ui.waitFor(() =>
    assert.ok(r.queryByText('Confirm bot enrollment') === null),
  );
  assert.ok(r.queryByText('Export token once') === null);
  ui.fireEvent.click(r.getByText('Check original enrollment'));
  await ui.waitFor(() => assert.equal(actions.length, 3));
  assert.equal(actions[2].action, 'status');
  r.rerender(
    createElement(BotPanel, {
      bridge,
      profile: 'local',
      account: 'other',
      onComplete: () => {},
    }),
  );
  await ui.waitFor(() =>
    assert.ok(r.queryByText('Check original enrollment') === null),
  );
});
