import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import { installDom } from './lib/dom-harness';
import {
  decodeInvitationReply,
  type InvitationAction,
  type InvitationReply,
} from '../src/invitation-contract';
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
test('invitation boundary rejects secret fields, bad handles and oversized inboxes', () => {
  for (const key of ['receipt', 'permission', 'seed', 'pin'])
    assert.throws(() => decodeInvitationReply({ rows: [{ [key]: 'secret' }] }));
  assert.throws(() => decodeInvitationReply({ operation_id: 'broken' }));
  assert.throws(() => decodeInvitationReply({ rows: Array(2001).fill({}) }));
  assert.deepEqual(
    decodeInvitationReply({
      state: 'submission-unknown',
      operation_id: 'a'.repeat(32),
    }),
    { state: 'submission-unknown', operation_id: 'a'.repeat(32) },
  );
});
test('preview precedes request preparation and unknown delivery is checked without replay', async () => {
  const { InvitationPanel } = (await vite.ssrLoadModule(
    '/src/components/invitation-panel.tsx',
  )) as typeof import('../src/components/invitation-panel');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const actions: string[] = [];
  const bridge = {
    ...mockBridge(),
    invitation: async (
      _p: string,
      _a: string,
      a: InvitationAction,
    ): Promise<InvitationReply> => {
      actions.push(a.action);
      if (a.action === 'preview')
        return {
          team_id: '3'.repeat(66),
          host_id: 'a'.repeat(66),
          name: 'project',
          membership: false,
        };
      return {
        operation_id: '1'.repeat(32),
        state: a.action === 'accept' ? 'prepared' : 'submission-unknown',
      };
    },
  };
  const r = ui.render(
    createElement(InvitationPanel, {
      bridge,
      profile: 'local',
      account: 'work',
      onComplete: () => assert.fail('no membership proof'),
    }),
  );
  assert.equal(
    (r.getByText('Prepare membership request') as HTMLButtonElement).disabled,
    true,
  );
  ui.fireEvent.change(r.getByLabelText('Invitation'), {
    target: { value: 'Invite123' },
  });
  ui.fireEvent.click(r.getByText('Preview invitation'));
  await ui.waitFor(() =>
    assert.equal(
      (r.getByText('Prepare membership request') as HTMLButtonElement).disabled,
      false,
    ),
  );
  ui.fireEvent.click(r.getByText('Prepare membership request'));
  await ui.waitFor(() => assert.ok(r.getByText('Submit prepared operation')));
  assert.deepEqual(actions, ['preview', 'accept']);
  ui.fireEvent.click(r.getByText('Submit prepared operation'));
  await ui.waitFor(() =>
    assert.ok(r.queryByText('Submit prepared operation') === null),
  );
  ui.fireEvent.click(r.getByText('Check operation'));
  await ui.waitFor(() =>
    assert.deepEqual(actions, ['preview', 'accept', 'attempt', 'status']),
  );
});
test('local certificate operations stay local while a remote inbox profile is selected', async () => {
  const { InvitationPanel } = (await vite.ssrLoadModule(
    '/src/components/invitation-panel.tsx',
  )) as typeof import('../src/components/invitation-panel');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const actions: string[] = [];
  const bridge = {
    ...mockBridge(),
    invitation: async (
      _p: string,
      _a: string,
      a: InvitationAction,
    ): Promise<InvitationReply> => {
      actions.push(a.action);
      return {
        operation_id: '2'.repeat(32),
        state: a.action === 'create' ? 'prepared' : 'complete',
      };
    },
  };
  const r = ui.render(
    createElement(InvitationPanel, {
      bridge,
      profile: 'local',
      account: 'work',
      teamAlias: 'project',
      onComplete: () => {},
    }),
  );
  ui.fireEvent.change(
    r.getByLabelText('Other server profile, for a remote request'),
    { target: { value: 'remote' } },
  );
  ui.fireEvent.click(r.getByText('Prepare invitation'));
  await ui.waitFor(() => assert.ok(r.getByText('Submit prepared operation')));
  ui.fireEvent.click(r.getByText('Submit prepared operation'));
  await ui.waitFor(() => assert.deepEqual(actions, ['create', 'attempt']));
});
