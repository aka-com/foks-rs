import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import { installDom } from './lib/dom-harness';
import { decodeRenameProgress } from '../src/rename-contract';
import type { RenameProgress } from '../src/rename-contract';
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
const progress: RenameProgress = {
  operation_id: '1'.repeat(32),
  account_alias: 'work',
  state: 'prepared',
  target: 'newname',
  current_username: 'oldname',
  hardware_required: false,
};
test('rename status rejects secrets, unknown fields and invalid handles', () => {
  assert.deepEqual(decodeRenameProgress([progress]), [progress]);
  for (const key of ['pin', 'seed', 'request'])
    assert.throws(() =>
      decodeRenameProgress([{ ...progress, [key]: 'secret' }]),
    );
  assert.throws(() =>
    decodeRenameProgress([{ ...progress, operation_id: 'invalid' }]),
  );
  assert.throws(() => decodeRenameProgress(Array(161).fill(progress)));
});
test('prepare requires explicit confirmation and uncertain outcomes are checked without replay', async () => {
  const { RenamePanel } = (await vite.ssrLoadModule(
    '/src/components/rename-panel.tsx',
  )) as typeof import('../src/components/rename-panel');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const actions: string[] = [];
  const bridge = {
    ...mockBridge(),
    renameAccount: async (
      _p: string,
      _a: string,
      action: import('../src/rename-contract').RenameAction | null,
    ): Promise<RenameProgress[]> => {
      actions.push(action?.action ?? 'list');
      return [
        {
          ...progress,
          state:
            action?.action === 'prepare' ? 'prepared' : 'submission-unknown',
        },
      ];
    },
  };
  const r = ui.render(
    createElement(RenamePanel, {
      bridge,
      profile: 'host',
      account: 'work',
      onComplete: () => {
        throw new Error('not completed');
      },
    }),
  );
  ui.fireEvent.change(r.getByLabelText('New username'), {
    target: { value: 'newname' },
  });
  ui.fireEvent.click(r.getByText('Prepare rename'));
  await ui.waitFor(() => assert.ok(r.getByText('Confirm rename')));
  assert.deepEqual(actions, ['prepare']);
  ui.fireEvent.click(r.getByText('Confirm rename'));
  await ui.waitFor(() => assert.ok(r.queryByText('Confirm rename') === null));
  ui.fireEvent.click(r.getByText('Check original operation'));
  await ui.waitFor(() =>
    assert.deepEqual(actions, ['prepare', 'attempt', 'status']),
  );
});
test('recover lists original handles and rejects another account response', async () => {
  const { RenamePanel } = (await vite.ssrLoadModule(
    '/src/components/rename-panel.tsx',
  )) as typeof import('../src/components/rename-panel');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const bridge = {
    ...mockBridge(),
    renameAccount: async () => [{ ...progress, account_alias: 'other' }],
  };
  const r = ui.render(
    createElement(RenamePanel, {
      bridge,
      profile: 'host',
      account: 'work',
      onComplete: () => {},
    }),
  );
  ui.fireEvent.click(r.getByText('Recover rename operations'));
  await ui.waitFor(() => assert.ok(r.getByRole('alert')));
  assert.ok(r.queryByText('Confirm rename') === null);
});
