import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactNode } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import { installDom } from './lib/dom-harness';
import { workflowScope } from './lib/workflow-scope';
import { decodeRenameProgress } from '../src/rename-contract';
import type { RenameProgress } from '../src/rename-contract';
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
  title: 'Change username',
  subtitle: 'ada on Example',
  onClose: () => {},
};
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
test('the dialog prepares a rename and returns its operation', async () => {
  const { RenamePanel } = (await vite.ssrLoadModule(
    '/src/components/rename-panel.tsx',
  )) as typeof import('../src/components/rename-panel');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const actions: string[] = [];
  const prepared: RenameProgress[][] = [];
  const bridge = {
    ...mockBridge(),
    renameAccount: async (
      _p: string,
      _a: string,
      action: import('../src/rename-contract').RenameAction | null,
    ): Promise<RenameProgress[]> => {
      actions.push(action?.action ?? 'list');
      return [progress];
    },
  };
  const r = ui.render(
    await overlay(
      createElement(RenamePanel, {
        bridge,
        profile: 'host',
        account: 'work',
        presentation,
        onPrepared: (rows) => {
          prepared.push(rows);
        },
      }),
    ),
  );
  // Pending-operation actions are on the account page.
  assert.equal(r.queryByText('Show pending changes'), null);
  assert.equal(r.queryByText('Confirm'), null);
  const prepare = r.getByRole('button', { name: 'Prepare rename' });
  assert.equal(prepare.hasAttribute('disabled'), true);
  ui.fireEvent.change(r.getByLabelText('Username'), {
    target: { value: 'newname' },
  });
  ui.fireEvent.click(prepare);
  await ui.waitFor(() => assert.deepEqual(prepared, [[progress]]));
  assert.deepEqual(actions, ['prepare']);
});
test('a prepare answered for another account is rejected', async () => {
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
    await overlay(
      createElement(RenamePanel, {
        bridge,
        profile: 'host',
        account: 'work',
        presentation,
        onPrepared: () => {
          throw new Error('not prepared');
        },
      }),
    ),
  );
  ui.fireEvent.change(r.getByLabelText('Username'), {
    target: { value: 'newname' },
  });
  ui.fireEvent.click(r.getByRole('button', { name: 'Prepare rename' }));
  await ui.waitFor(() => assert.ok(r.getByRole('alert')));
});
