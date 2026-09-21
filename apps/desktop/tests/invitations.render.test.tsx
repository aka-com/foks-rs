import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactNode } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import { installDom } from './lib/dom-harness';
import {
  decodeInvitationReply,
  type InvitationAction,
  type InvitationReply,
} from '../src/invitation-contract';
import { INVITATION_ACTIVITY } from '../src/invitation-activity';
import type { TeamStore } from '../src/model';
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
    children,
  });
}

const presentation = {
  title: 'Join a group',
  subtitle: 'ada on Example',
  onClose: () => {},
};
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
test('the invitation activity band stays quiet about a read the profile could not admit', async () => {
  const { InvitationRecovery } = (await vite.ssrLoadModule(
    '/src/components/invitation-recovery.tsx',
  )) as typeof import('../src/components/invitation-recovery');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const { FIXTURE } = (await vite.ssrLoadModule(
    '/src/fixture.ts',
  )) as typeof import('../src/fixture');
  const team = FIXTURE.stores.find(
    (store): store is TeamStore => store.kind === 'team',
  );
  assert.ok(team);
  // The read is queued on the profile; work ahead of it that never finishes
  // expires it at the admission deadline, and the agent being busy reads
  // the same way. Neither is the user's to hear about from a badge.
  let failure: unknown = {
    code: 'profile-busy',
    message: 'Profile queue deadline exceeded; request did not start.',
    retryable: true,
    fatal: false,
    ambiguous: false,
  };
  const bridge = {
    ...mockBridge(FIXTURE),
    invitation: async (): Promise<InvitationReply> => {
      throw failure;
    },
  };
  const errors: unknown[] = [];
  const rendered = ui.render(
    await overlay(
      createElement(InvitationRecovery, {
        bridge,
        store: team,
        onReview: () => {},
        onError: (error: unknown) => errors.push(error),
      }),
    ),
  );
  await ui.act(async () => {});
  assert.deepEqual(errors, []);
  assert.equal(rendered.queryByText('Invitation activity'), null);
  // What changes the session still reaches the handler.
  failure = {
    code: 'agent-lost',
    message: 'The agent stopped.',
    retryable: false,
    fatal: true,
    ambiguous: false,
  };
  await ui.act(async () => {
    window.dispatchEvent(
      new window.CustomEvent(INVITATION_ACTIVITY, {
        detail: { profile: team.server, account: team.account },
      }),
    );
  });
  assert.equal(errors.length, 1);
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
    await overlay(
      createElement(InvitationPanel, {
        bridge,
        profile: 'local',
        account: 'work',
        presentation,
        onComplete: () => assert.fail('no membership proof'),
      }),
    ),
  );
  assert.equal(
    (r.getByText('Request membership') as HTMLButtonElement).disabled,
    true,
  );
  ui.fireEvent.change(r.getByLabelText('Invitation'), {
    target: { value: 'Invite123' },
  });
  ui.fireEvent.click(r.getByText('Preview'));
  await ui.waitFor(() =>
    assert.equal(
      (r.getByText('Request membership') as HTMLButtonElement).disabled,
      false,
    ),
  );
  ui.fireEvent.click(r.getByText('Request membership'));
  await ui.waitFor(() => assert.ok(r.getByText('Submit')));
  assert.deepEqual(actions, ['preview', 'accept']);
  ui.fireEvent.click(r.getByText('Submit'));
  await ui.waitFor(() => assert.ok(r.queryByText('Submit') === null));
  ui.fireEvent.click(r.getByText('Check status'));
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
  ui.fireEvent.change(r.getByLabelText('Server profile'), {
    target: { value: 'remote' },
  });
  ui.fireEvent.click(r.getByText('Create invitation'));
  await ui.waitFor(() => assert.ok(r.getByText('Submit')));
  ui.fireEvent.click(r.getByText('Submit'));
  // The team side loads its own request list up front, so mounting is
  // itself an "inbox" read before the person under test does anything.
  await ui.waitFor(() =>
    assert.deepEqual(actions, ['inbox', 'create', 'attempt']),
  );
});
