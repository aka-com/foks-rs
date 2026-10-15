import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactNode } from 'react';
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
test('the unfinished activity band stays quiet about a read the profile could not admit', async () => {
  const { UnfinishedActivity } =
    await import('../src/components/unfinished-activity');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const { FIXTURE } = await import('../src/mock/fixture');
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
      createElement(UnfinishedActivity, {
        bridge,
        store: team,
        membership: [],
        invitations: true,
        onReview: () => {},
        onError: (error: unknown) => errors.push(error),
      }),
    ),
  );
  await ui.act(async () => {});
  assert.deepEqual(errors, []);
  assert.equal(rendered.queryByText('Unfinished activity'), null);
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

test('the join sheet confirms the resolved team, then prepares and sends in one press', async () => {
  const { InvitationPanel } =
    await import('../src/components/invitation-panel');
  const { mockBridge } = await import('../src/mock/mock-bridge');
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
  // Step one commits to nothing: the request cannot be reached, and the step
  // cannot be left, until an invitation is typed.
  assert.equal(r.queryByText('Request membership'), null);
  assert.ok(r.getByText('Step 1 of 3'));
  assert.equal((r.getByText('Continue') as HTMLButtonElement).disabled, true);
  ui.fireEvent.change(r.getByLabelText('Invitation'), {
    target: { value: 'Invite123' },
  });
  assert.equal((r.getByText('Continue') as HTMLButtonElement).disabled, false);
  ui.fireEvent.click(r.getByText('Continue'));
  // Step two displays the resolved team and defaults the requester to the
  // current account.
  await ui.waitFor(() => assert.ok(r.getByText('Step 2 of 3')));
  assert.ok(r.getByText('project'));
  assert.ok(r.getByText('a'.repeat(66)));
  assert.equal(
    r.getByRole('radio', { name: /Myself/ }).getAttribute('aria-checked'),
    'true',
  );
  assert.equal(r.queryByLabelText('Team to add'), null);
  assert.equal(
    (r.getByText('Request membership') as HTMLButtonElement).disabled,
    false,
  );
  // Back returns to the invitation without sending anything.
  ui.fireEvent.click(r.getByText('Back'));
  assert.ok(r.getByText('Step 1 of 3'));
  assert.deepEqual(actions, ['preview']);
  ui.fireEvent.click(r.getByText('Continue'));
  await ui.waitFor(() => assert.ok(r.getByText('Request membership')));
  // One action prepares and submits the request. Step three displays the
  // submission-unknown response used by this test.
  ui.fireEvent.click(r.getByText('Request membership'));
  await ui.waitFor(() => assert.ok(r.getByText('Step 3 of 3')));
  assert.deepEqual(actions, ['preview', 'preview', 'accept', 'attempt']);
  assert.ok(r.getByText(/server response was not received/));
  ui.fireEvent.click(r.getByText('Check status'));
  await ui.waitFor(() =>
    assert.deepEqual(actions, [
      'preview',
      'preview',
      'accept',
      'attempt',
      'status',
    ]),
  );
});
test('a team the account administers is picked from a list and sent by its alias', async () => {
  const { InvitationPanel } =
    await import('../src/components/invitation-panel');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const seen: InvitationAction[] = [];
  const bridge = {
    ...mockBridge(),
    invitation: async (
      _p: string,
      _a: string,
      a: InvitationAction,
    ): Promise<InvitationReply> => {
      seen.push(a);
      if (a.action === 'preview')
        return { team_id: '3'.repeat(66), host_id: 'a'.repeat(66) };
      return { operation_id: '1'.repeat(32), state: 'complete' };
    },
  };
  const r = ui.render(
    await overlay(
      createElement(InvitationPanel, {
        bridge,
        profile: 'local',
        account: 'work',
        teams: [
          { id: 'team:infra', alias: 'infra', name: 'Infrastructure' },
          { id: 'team:platform', alias: 'platform', name: 'Platform' },
        ],
        presentation,
        onComplete: () => {},
      }),
    ),
  );
  ui.fireEvent.change(r.getByLabelText('Invitation'), {
    target: { value: 'Invite123' },
  });
  ui.fireEvent.click(r.getByText('Continue'));
  await ui.waitFor(() => assert.ok(r.getByText('Step 2 of 3')));
  ui.fireEvent.click(r.getByRole('radio', { name: /A team I administer/ }));
  // Selecting team-based membership requires a specific team.
  assert.equal(
    (r.getByText('Request membership') as HTMLButtonElement).disabled,
    true,
  );
  const picker = r.getByLabelText('Team to add') as HTMLSelectElement;
  assert.deepEqual(
    [...picker.options].map((option) => option.textContent),
    ['Choose a team', 'Infrastructure', 'Platform'],
  );
  ui.fireEvent.change(picker, { target: { value: 'platform' } });
  ui.fireEvent.change(r.getByLabelText('Your role in it'), {
    target: { value: 'owner' },
  });
  ui.fireEvent.click(r.getByText('Request membership'));
  await ui.waitFor(() => assert.ok(r.getByText('Step 3 of 3')));
  assert.deepEqual(seen.at(-1), {
    action: 'accept-team',
    invite: 'Invite123',
    source_team_alias: 'platform',
    source_role: 'owner',
  });
  assert.ok(r.getByText('Platform (owner)'));
});
test('the join sheet names a configured profile rather than asking for one', async () => {
  const { InvitationPanel } =
    await import('../src/components/invitation-panel');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const { FIXTURE } = await import('../src/mock/fixture');
  const seen: InvitationAction[] = [];
  const bridge = {
    ...mockBridge(FIXTURE),
    invitation: async (
      _p: string,
      _a: string,
      a: InvitationAction,
    ): Promise<InvitationReply> => {
      seen.push(a);
      return { team_id: '3'.repeat(66), host_id: 'a'.repeat(66) };
    },
  };
  const other = FIXTURE.servers.find(
    (server) => server.profileName !== 'local',
  );
  assert.ok(other);
  const r = ui.render(
    await overlay(
      createElement(InvitationPanel, {
        bridge,
        profile: 'local',
        account: 'work',
        servers: FIXTURE.servers,
        presentation,
        onComplete: () => {},
      }),
    ),
  );
  ui.fireEvent.change(r.getByLabelText('Invitation'), {
    target: { value: 'Invite123' },
  });
  const picker = r.getByLabelText(
    'Where is the team located?',
  ) as HTMLSelectElement;
  // The account's own profile is not a remote one, so it is never offered.
  assert.ok(
    ![...picker.options].some((option) => option.value === 'local'),
    'the home profile is offered as a remote one',
  );
  ui.fireEvent.change(picker, { target: { value: other.profileName } });
  ui.fireEvent.click(r.getByText('Continue'));
  await ui.waitFor(() => assert.equal(seen.length, 1));
  assert.deepEqual(seen[0], {
    action: 'preview-remote',
    remote_profile: other.profileName,
    invite: 'Invite123',
  });
});
test('Invite new user prepares and submits in one press, then shares the token', async () => {
  const { InviteNewUserSheet } =
    await import('../src/components/invite-new-user-sheet');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const { FIXTURE } = await import('../src/mock/fixture');
  const team = FIXTURE.stores.find(
    (store): store is TeamStore => store.kind === 'team',
  );
  assert.ok(team);
  const actions: string[] = [];
  const bridge = {
    ...mockBridge(FIXTURE),
    invitation: async (
      _p: string,
      _a: string,
      a: InvitationAction,
    ): Promise<InvitationReply> => {
      actions.push(a.action);
      if (a.action === 'list') return [];
      return {
        operation_id: '2'.repeat(32),
        state: a.action === 'create' ? 'prepared' : 'complete',
        ...(a.action === 'attempt' ? { invite: 'Invite456' } : {}),
      };
    },
  };
  const r = ui.render(
    await overlay(
      createElement(InviteNewUserSheet, {
        bridge,
        team,
        serverLabel: 'Example',
        approver: 'ada',
        onClose: () => {},
      }),
    ),
  );
  assert.ok(r.getByText('Step 1 of 2'));
  ui.fireEvent.change(r.getByLabelText('For'), {
    target: { value: 'Sam, new contractor' },
  });
  ui.fireEvent.click(r.getByText('Create invitation'));
  await ui.waitFor(() => assert.ok(r.getByLabelText('Shareable invitation')));
  assert.deepEqual(actions, ['list', 'create', 'attempt']);
  assert.ok(r.getByText('Step 2 of 2'));
  assert.equal(
    r.getByLabelText('Shareable invitation').textContent,
    'Invite456',
  );
  // The copied instructions must include the server, invitation token, and approver.
  const instructions =
    document.querySelector('.copybox pre')?.textContent ?? '';
  assert.match(instructions, /Server: Example/);
  assert.match(instructions, /Invite456/);
  assert.match(instructions, /approved by ada/);
});

test('Invite new user asks for the security key PIN only when the key requires it', async () => {
  const { InviteNewUserSheet } =
    await import('../src/components/invite-new-user-sheet');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const { FIXTURE } = await import('../src/mock/fixture');
  const team = FIXTURE.stores.find(
    (store): store is TeamStore => store.kind === 'team',
  );
  assert.ok(team);
  const received: Array<[string, string | null]> = [];
  const operation = { operation_id: 'c'.repeat(32), state: 'prepared' };
  const bridge = {
    ...mockBridge(FIXTURE),
    invitation: async (
      _p: string,
      _a: string,
      a: InvitationAction,
      pin: string | null,
    ): Promise<InvitationReply> => {
      if (a.action === 'list') return [];
      received.push([a.action, pin]);
      if (a.action === 'create') return operation;
      return pin
        ? { ...operation, state: 'complete', invite: 'Invite789' }
        : { ...operation, hardware_required: true };
    },
  };
  const r = ui.render(
    await overlay(
      createElement(InviteNewUserSheet, {
        bridge,
        team,
        serverLabel: 'Example',
        onClose: () => {},
      }),
    ),
  );
  assert.equal(r.queryByLabelText('Security key PIN'), null);
  ui.fireEvent.click(r.getByText('Create invitation'));
  await ui.waitFor(() => assert.ok(r.getByLabelText('Security key PIN')));
  assert.deepEqual(received, [
    ['create', null],
    ['attempt', null],
  ]);
  ui.fireEvent.change(r.getByLabelText('Security key PIN'), {
    target: { value: '123456' },
  });
  ui.fireEvent.click(r.getByText('Finish creating'));
  await ui.waitFor(() => assert.ok(r.getByLabelText('Shareable invitation')));
  assert.deepEqual(received.at(-1), ['attempt', '123456']);
});

test('Invite new user keeps a PIN refusal and states the attempts left', async () => {
  const { InviteNewUserSheet } =
    await import('../src/components/invite-new-user-sheet');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const { FIXTURE } = await import('../src/mock/fixture');
  const team = FIXTURE.stores.find(
    (store): store is TeamStore => store.kind === 'team',
  );
  assert.ok(team);
  const received: Array<string | null> = [];
  const operation = { operation_id: 'd'.repeat(32), state: 'prepared' };
  // The agent reports a refused PIN as `operation-failed` with the
  // foks_yubi error text.
  const refusals = [
    'Hardware key PIN was rejected; 1 attempt(s) remain',
    'Hardware key PIN is blocked',
  ];
  const bridge = {
    ...mockBridge(FIXTURE),
    invitation: async (
      _p: string,
      _a: string,
      a: InvitationAction,
      pin: string | null,
    ): Promise<InvitationReply> => {
      if (a.action === 'list') return [];
      if (a.action === 'create') return operation;
      received.push(pin);
      if (!pin) return { ...operation, hardware_required: true };
      throw {
        code: 'operation-failed',
        message: refusals.shift(),
        retryable: false,
        ambiguous: false,
        fatal: false,
      };
    },
  };
  const r = ui.render(
    await overlay(
      createElement(InviteNewUserSheet, {
        bridge,
        team,
        serverLabel: 'Example',
        onClose: () => {},
      }),
    ),
  );
  ui.fireEvent.click(r.getByText('Create invitation'));
  await ui.waitFor(() => assert.ok(r.getByLabelText('Security key PIN')));
  const finish = () => r.getByText('Finish creating').closest('button')!;

  // A value the card would refuse is named and not sent.
  ui.fireEvent.change(r.getByLabelText('Security key PIN'), {
    target: { value: '12345' },
  });
  assert.ok(r.getByText('Use 6 to 8 characters.'));
  assert.equal(finish().disabled, true);

  ui.fireEvent.change(r.getByLabelText('Security key PIN'), {
    target: { value: '111111' },
  });
  assert.equal(finish().disabled, false);
  ui.fireEvent.click(finish());
  await ui.waitFor(() =>
    assert.equal(
      r.getByRole('alert').textContent,
      'Hardware key PIN was rejected; 1 attempt(s) remain',
    ),
  );
  const note = r.getByText(/^1 attempt left\. Another incorrect PIN blocks/);
  assert.equal(
    r.getByLabelText('Security key PIN').getAttribute('aria-describedby'),
    note.id,
  );
  // The refused PIN is cleared, so it cannot be sent again unchanged.
  assert.equal(
    (r.getByLabelText('Security key PIN') as HTMLInputElement).value,
    '',
  );
  assert.equal(finish().disabled, true);

  ui.fireEvent.change(r.getByLabelText('Security key PIN'), {
    target: { value: '222222' },
  });
  ui.fireEvent.click(finish());
  await ui.waitFor(() =>
    assert.equal(
      r.getByRole('alert').textContent,
      'Hardware key PIN is blocked',
    ),
  );
  assert.ok(r.getByText(/^The PIN is blocked\. Unblock it with the PUK/));
  assert.deepEqual(received, [null, '111111', '222222']);
});

test('a resumed hardware-key request can receive its PIN on step one', async () => {
  const { InvitationPanel } =
    await import('../src/components/invitation-panel');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const { FIXTURE } = await import('../src/mock/fixture');
  const received: Array<string | null> = [];
  const operation = { operation_id: 'a'.repeat(32), state: 'prepared' };
  const bridge = {
    ...mockBridge(FIXTURE),
    invitation: async (
      _p: string,
      _a: string,
      action: InvitationAction,
      pin: string | null,
    ) => {
      if (action.action === 'list') return [operation];
      received.push(pin);
      return pin
        ? { ...operation, state: 'complete' }
        : { ...operation, hardware_required: true };
    },
  };
  const r = ui.render(
    await overlay(
      createElement(InvitationPanel, {
        bridge,
        profile: 'local',
        account: 'work',
        presentation,
        onComplete: () => {},
      }),
    ),
  );
  ui.fireEvent.click(r.getByText('Resume a pending request'));
  await ui.waitFor(() => assert.ok(r.getByText('Submit')));
  ui.fireEvent.click(r.getByText('Submit'));
  await ui.waitFor(() => assert.equal(received.length, 1));
  assert.equal(received[0], null);
  ui.fireEvent.change(r.getByLabelText('Security key PIN'), {
    target: { value: '123456' },
  });
  ui.fireEvent.click(r.getByText('Submit'));
  await ui.waitFor(() => assert.deepEqual(received, [null, '123456']));
  await ui.waitFor(() => assert.equal(r.queryByText('Submit'), null));
});

test('a refused PIN on a resumed request is cleared and not sent again', async () => {
  const { InvitationPanel } =
    await import('../src/components/invitation-panel');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const { FIXTURE } = await import('../src/mock/fixture');
  const received: Array<string | null> = [];
  const operation = { operation_id: 'e'.repeat(32), state: 'prepared' };
  const bridge = {
    ...mockBridge(FIXTURE),
    invitation: async (
      _p: string,
      _a: string,
      action: InvitationAction,
      pin: string | null,
    ) => {
      if (action.action === 'list') return [operation];
      received.push(pin);
      if (!pin) return { ...operation, hardware_required: true };
      throw {
        code: 'operation-failed',
        message: 'Hardware key PIN was rejected; 1 attempt(s) remain',
        retryable: false,
        ambiguous: false,
        fatal: false,
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
        onComplete: () => {},
      }),
    ),
  );
  ui.fireEvent.click(r.getByText('Resume a pending request'));
  await ui.waitFor(() => assert.ok(r.getByText('Submit')));
  ui.fireEvent.click(r.getByText('Submit'));
  await ui.waitFor(() => assert.ok(r.getByLabelText('Security key PIN')));
  ui.fireEvent.change(r.getByLabelText('Security key PIN'), {
    target: { value: '111111' },
  });
  ui.fireEvent.click(r.getByText('Submit'));
  await ui.waitFor(() =>
    assert.equal(
      r.getByRole('alert').textContent,
      'Hardware key PIN was rejected; 1 attempt(s) remain',
    ),
  );
  assert.ok(r.getByText(/^1 attempt left\. Another incorrect PIN blocks/));
  assert.equal(
    (r.getByLabelText('Security key PIN') as HTMLInputElement).value,
    '',
  );
  // Submitting again without a new PIN does not resend the refused one.
  ui.fireEvent.click(r.getByText('Submit'));
  await ui.waitFor(() => assert.equal(received.length, 3));
  assert.deepEqual(received, [null, '111111', null]);
});

test('an invitation action discards the profile’s journal of unfinished operations', async () => {
  const { InvitationPanel } =
    await import('../src/components/invitation-panel');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const { MetadataRepository } =
    await import('../src/data/metadata-repository');
  const { MetadataRepositoryContext } = await import('../src/data/query-hooks');
  const { pendingOperationsQuery } =
    await import('../src/data/operation-queries');
  let now = 0;
  let reads = 0;
  const repository = new MetadataRepository(() => now);
  const bridge = {
    ...mockBridge(),
    listPendingOperations: async () => {
      reads++;
      return [];
    },
    invitation: async (
      _p: string,
      _a: string,
      a: InvitationAction,
    ): Promise<InvitationReply> =>
      a.action === 'preview'
        ? {
            team_id: '3'.repeat(66),
            host_id: 'a'.repeat(66),
            name: 'project',
            membership: false,
          }
        : { operation_id: '1'.repeat(32), state: 'prepared' },
  };
  // The journal a team page open elsewhere is holding.
  const journal = pendingOperationsQuery(repository, bridge, 'local');
  await journal.load();
  assert.equal(reads, 1);
  const r = ui.render(
    await overlay(
      createElement(MetadataRepositoryContext.Provider, {
        value: repository,
        children: createElement(InvitationPanel, {
          bridge,
          profile: 'local',
          account: 'work',
          presentation,
          onComplete: () => {},
        }),
      }),
    ),
  );
  ui.fireEvent.change(r.getByLabelText('Invitation'), {
    target: { value: 'Invite123' },
  });
  ui.fireEvent.click(r.getByText('Continue'));
  await ui.waitFor(() =>
    assert.equal(
      (r.getByText('Request membership') as HTMLButtonElement).disabled,
      false,
    ),
  );
  ui.fireEvent.click(r.getByText('Request membership'));
  await ui.waitFor(() => assert.ok(r.getByText('Submit')));
  // Well inside the journal's freshness window, so only the invalidation the
  // admission made can be what reads it again.
  now = 1_000;
  await journal.load();
  assert.equal(reads, 2);
});

test('remote requests select a configured server before verification and approval', async () => {
  const { MembershipRequests } =
    await import('../src/components/membership-requests');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const { FIXTURE } = await import('../src/mock/fixture');
  const team = FIXTURE.stores.find(
    (store): store is TeamStore => store.kind === 'team',
  );
  assert.ok(team);
  const remoteServers = FIXTURE.servers.filter(
    (server) => server.profileName !== team.server,
  );
  assert.ok(remoteServers.length >= 2);
  const request = {
    request_id: 'a'.repeat(32),
    username: 'visitor',
    remote: true,
    verified: false,
  };
  const actions: InvitationAction[] = [];
  const bridge = {
    ...mockBridge(FIXTURE),
    invitation: async (
      _p: string,
      _a: string,
      action: InvitationAction,
    ): Promise<InvitationReply> => {
      actions.push(action);
      if (action.action === 'inbox') return { rows: [request] };
      if (action.action === 'list') return [];
      if (action.action === 'inspect-remote')
        return {
          ...request,
          verified: true,
          source_profile: action.remote_profile,
        };
      return { state: 'complete' };
    },
  };
  const r = ui.render(
    await overlay(
      createElement(MembershipRequests, {
        bridge,
        team,
        servers: FIXTURE.servers,
        serverLabel: 'Example',
        onComplete() {},
        onInvite() {},
        onActivity() {},
      }),
    ),
  );
  await ui.waitFor(() => assert.ok(r.getByText('Verify')));
  assert.equal((r.getByText('Verify') as HTMLButtonElement).disabled, true);
  ui.fireEvent.change(r.getByLabelText('Server for visitor'), {
    target: { value: remoteServers[0].profileName },
  });
  ui.fireEvent.click(r.getByText('Verify'));
  await ui.waitFor(() => assert.ok(r.getByText('Approve')));
  assert.deepEqual(actions.at(-1), {
    action: 'inspect-remote',
    remote_profile: remoteServers[0].profileName,
    team_alias: team.alias,
    request_id: request.request_id,
  });
  // Require verification on the newly selected server before allowing approval.
  ui.fireEvent.change(r.getByLabelText('Server for visitor'), {
    target: { value: remoteServers[1].profileName },
  });
  assert.equal(r.queryByText('Approve'), null);
  ui.fireEvent.click(r.getByText('Verify'));
  await ui.waitFor(() => assert.ok(r.getByText('Approve')));
  ui.fireEvent.click(r.getByText('Approve'));
  await ui.waitFor(() =>
    assert.ok(actions.some((action) => action.action === 'approve-remote')),
  );
  assert.deepEqual(
    actions.find((action) => action.action === 'approve-remote'),
    {
      action: 'approve-remote',
      remote_profile: remoteServers[1].profileName,
      team_alias: team.alias,
      request_id: request.request_id,
      role: { member: { visibility: 0 } },
    },
  );
});

test('hardware inbox loads prompt for a PIN and use it only on explicit refresh', async () => {
  const { MembershipRequests } =
    await import('../src/components/membership-requests');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const { FIXTURE } = await import('../src/mock/fixture');
  const team = FIXTURE.stores.find(
    (store): store is TeamStore => store.kind === 'team',
  );
  assert.ok(team);
  const pins: Array<string | null> = [];
  const bridge = {
    ...mockBridge(FIXTURE),
    invitation: async (
      _p: string,
      _a: string,
      action: InvitationAction,
      pin: string | null,
    ): Promise<InvitationReply> => {
      if (action.action !== 'inbox') return [];
      pins.push(pin);
      if (!pin) throw new Error('unlock the account key for invitations');
      return {
        rows: [
          { request_id: 'b'.repeat(32), username: 'visitor', verified: true },
        ],
      };
    },
  };
  const r = ui.render(
    await overlay(
      createElement(MembershipRequests, {
        bridge,
        team,
        servers: FIXTURE.servers,
        serverLabel: 'Example',
        onComplete() {},
        onInvite() {},
        onActivity() {},
      }),
    ),
  );
  await ui.waitFor(() => assert.ok(r.getByLabelText('Security key PIN')));
  ui.fireEvent.change(r.getByLabelText('Security key PIN'), {
    target: { value: '123456' },
  });
  assert.deepEqual(pins, [null]);
  ui.fireEvent.click(r.getByText('Refresh'));
  await ui.waitFor(() => assert.ok(r.getByText('Approve')));
  assert.deepEqual(pins, [null, '123456']);
  assert.equal(r.queryByLabelText('Security key PIN'), null);
  ui.fireEvent.click(r.getByText('Refresh'));
  await ui.waitFor(() => assert.ok(r.getByLabelText('Security key PIN')));
  assert.deepEqual(pins, [null, '123456', null]);
});

test('a failed invitation submission retries the original operation', async () => {
  const { InviteNewUserSheet } =
    await import('../src/components/invite-new-user-sheet');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const { FIXTURE } = await import('../src/mock/fixture');
  const team = FIXTURE.stores.find(
    (store): store is TeamStore => store.kind === 'team',
  );
  assert.ok(team);
  const operation = { operation_id: 'c'.repeat(32), state: 'prepared' };
  const actions: InvitationAction[] = [];
  let attempts = 0;
  const bridge = {
    ...mockBridge(FIXTURE),
    invitation: async (
      _p: string,
      _a: string,
      action: InvitationAction,
    ): Promise<InvitationReply> => {
      actions.push(action);
      if (action.action === 'list') return [];
      if (action.action === 'create') return operation;
      if (++attempts === 1) throw new Error('Connection lost');
      return { ...operation, state: 'complete', invite: 'RecoveredInvite' };
    },
  };
  const r = ui.render(
    await overlay(
      createElement(InviteNewUserSheet, {
        bridge,
        team,
        serverLabel: 'Example',
        onClose() {},
      }),
    ),
  );
  ui.fireEvent.click(r.getByText('Create invitation'));
  await ui.waitFor(() => assert.ok(r.getByText('Connection lost')));
  ui.fireEvent.click(r.getByText('Finish creating'));
  await ui.waitFor(() =>
    assert.equal(
      r.getByLabelText('Shareable invitation').textContent,
      'RecoveredInvite',
    ),
  );
  assert.equal(
    actions.filter((action) => action.action === 'create').length,
    1,
  );
  assert.deepEqual(
    actions.filter((action) => action.action === 'attempt'),
    [
      { action: 'attempt', operation_id: operation.operation_id },
      { action: 'attempt', operation_id: operation.operation_id },
    ],
  );
});

for (const state of ['prepared', 'submission-unknown'] as const) {
  test(`resuming a saved ${state} invitation retains its identity after a connection failure`, async () => {
    const actionName = state === 'prepared' ? 'attempt' : 'status';
    const { InviteNewUserSheet } =
      await import('../src/components/invite-new-user-sheet');
    const { mockBridge } = await import('../src/mock/mock-bridge');
    const { FIXTURE } = await import('../src/mock/fixture');
    const team = FIXTURE.stores.find(
      (store): store is TeamStore => store.kind === 'team',
    );
    assert.ok(team);
    const operation = {
      operation_id: 'd'.repeat(32),
      team_id: team.team_id_hex,
      state,
    };
    const actions: InvitationAction[] = [];
    let attempts = 0;
    const bridge = {
      ...mockBridge(FIXTURE),
      invitation: async (_p: string, _a: string, action: InvitationAction) => {
        actions.push(action);
        if (action.action === 'list') return [operation];
        assert.equal(action.action, actionName);
        if (++attempts === 1)
          throw new Error("Can't assign requested address (os error 49)");
        return { ...operation, state: 'complete', invite: 'RecoveredInvite' };
      },
    };
    const r = ui.render(
      await overlay(
        createElement(InviteNewUserSheet, {
          bridge,
          team,
          serverLabel: 'Example',
          onClose() {},
        }),
      ),
    );
    await ui.waitFor(() => assert.ok(r.getByText('Invitation not finished')));
    ui.fireEvent.click(
      r.getByText(state === 'prepared' ? 'Finish creating' : 'Check status'),
    );
    await ui.waitFor(() => assert.ok(r.getByRole('alert')));
    ui.fireEvent.click(r.getByText('Finish creating'));
    await ui.waitFor(() => assert.ok(r.getByLabelText('Shareable invitation')));
    assert.deepEqual(actions, [
      { action: 'list' },
      { action: actionName, operation_id: operation.operation_id },
      { action: actionName, operation_id: operation.operation_id },
    ]);
  });
}

test('an acknowledged invitation is finalized and refresh failure preserves successful creation', async () => {
  const { InviteNewUserSheet } =
    await import('../src/components/invite-new-user-sheet');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const { FIXTURE } = await import('../src/mock/fixture');
  const team = FIXTURE.stores.find(
    (store): store is TeamStore => store.kind === 'team',
  );
  assert.ok(team);
  const operation = { operation_id: 'e'.repeat(32), state: 'acknowledged' };
  const actions: InvitationAction[] = [];
  const bridge = {
    ...mockBridge(FIXTURE),
    invitation: async (_p: string, _a: string, action: InvitationAction) => {
      actions.push(action);
      if (action.action === 'list') return [];
      if (action.action === 'create') return operation;
      assert.equal(action.action, 'status');
      return { ...operation, state: 'complete', invite: 'ReadyInvite' };
    },
  };
  const r = ui.render(
    await overlay(
      createElement(InviteNewUserSheet, {
        bridge,
        team,
        serverLabel: 'Example',
        onClose() {},
        async onComplete() {
          throw new Error('Connection lost');
        },
      }),
    ),
  );
  ui.fireEvent.click(r.getByText('Create invitation'));
  await ui.waitFor(() =>
    assert.match(
      r.getByRole('alert').textContent ?? '',
      /invitation is ready, but refreshing the team failed: Connection lost/,
    ),
  );
  assert.equal(
    r.getByLabelText('Shareable invitation').textContent,
    'ReadyInvite',
  );
  assert.equal((r.getByText('Done') as HTMLButtonElement).disabled, false);
  assert.deepEqual(actions, [
    { action: 'list' },
    { action: 'create', team_alias: team.alias },
    { action: 'status', operation_id: operation.operation_id },
  ]);
});

test('delivered requests remain reachable pending membership approval', async () => {
  const { InvitationPanel } =
    await import('../src/components/invitation-panel');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const bridge = {
    ...mockBridge(),
    invitation: async () => [
      {
        operation_id: '1'.repeat(32),
        team_id: '3'.repeat(66),
        state: 'complete',
        delivery_acknowledged: true,
      },
    ],
  };
  const r = ui.render(
    await overlay(
      createElement(InvitationPanel, {
        bridge,
        profile: 'local',
        account: 'ada',
        presentation,
        onComplete: () => {},
      }),
    ),
  );
  ui.fireEvent.click(r.getByText('Resume a pending request'));
  await ui.waitFor(() => assert.ok(r.queryByText('Check status')));
});

for (const remoteRequest of [false, true]) {
  for (const state of ['prepared', 'submission-unknown']) {
    test(`${remoteRequest ? 'remote' : 'local'} saved ${state} request keeps its destination across retries and status replies`, async () => {
      const { InvitationPanel } =
        await import('../src/components/invitation-panel');
      const { mockBridge } = await import('../src/mock/mock-bridge');
      const actions: InvitationAction[] = [];
      const row = {
        operation_id: '1'.repeat(32),
        team_id: '3'.repeat(66),
        state,
        remote: remoteRequest,
        ...(remoteRequest ? { remote_profile: 'destination' } : {}),
      };
      const bridge = {
        ...mockBridge(),
        invitation: async (
          _p: string,
          _a: string,
          action: InvitationAction,
        ) => {
          actions.push(action);
          // Native progress replies need not repeat destination metadata.
          return action.action === 'list'
            ? [row]
            : { operation_id: row.operation_id, state: 'submission-unknown' };
        },
      };
      const r = ui.render(
        await overlay(
          createElement(InvitationPanel, {
            bridge,
            profile: 'local',
            account: 'ada',
            presentation,
            onComplete: () => {},
          }),
        ),
      );
      ui.fireEvent.change(
        r.getByPlaceholderText('For teams on another server'),
        { target: { value: 'unrelated-server' } },
      );
      ui.fireEvent.click(r.getByText('Resume a pending request'));
      await ui.waitFor(() =>
        assert.ok(
          r.queryByText(state === 'prepared' ? 'Submit' : 'Check status'),
        ),
      );
      await ui.act(async () => {
        ui.fireEvent.click(
          r.getByText(state === 'prepared' ? 'Submit' : 'Check status'),
        );
      });
      await ui.act(async () => {
        ui.fireEvent.click(r.getByText('Check status'));
      });
      const operation = remoteRequest
        ? { operation_id: row.operation_id, remote_profile: 'destination' }
        : { operation_id: row.operation_id };
      assert.deepEqual(actions.slice(1), [
        {
          action: `${state === 'prepared' ? 'attempt' : 'status'}${remoteRequest ? '-remote' : ''}`,
          ...operation,
        },
        { action: remoteRequest ? 'status-remote' : 'status', ...operation },
      ]);
    });
  }
}

test('requests load independently, show truncation and use millisecond ages', async () => {
  const { MembershipRequests } =
    await import('../src/components/membership-requests');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const { FIXTURE } = await import('../src/mock/fixture');
  const team = FIXTURE.stores.find(
    (store): store is TeamStore => store.kind === 'team',
  );
  assert.ok(team);
  let failList: ((error: Error) => void) | undefined;
  let inboxReads = 0;
  const bridge = {
    ...mockBridge(FIXTURE),
    invitation: async (
      _p: string,
      _a: string,
      action: InvitationAction,
    ): Promise<InvitationReply> => {
      if (action.action === 'list')
        return new Promise((_, reject) => {
          failList = reject;
        });
      if (action.action === 'inbox')
        return {
          rows: [
            {
              request_id: 'a'.repeat(32),
              username: `visitor${++inboxReads}`,
              verified: true,
              time: Date.now() - 86_400_000,
            },
          ],
          possibly_truncated: true,
        };
      return { state: 'complete' };
    },
  };
  const r = ui.render(
    await overlay(
      createElement(MembershipRequests, {
        bridge,
        team,
        servers: FIXTURE.servers,
        serverLabel: 'Example',
        onComplete() {},
        onInvite() {},
        onActivity() {},
      }),
    ),
  );
  // The operation history is still pending, but the inbox is already usable.
  await ui.waitFor(() => assert.ok(r.getByText('visitor1')));
  assert.ok(r.getByText(/yesterday/));
  assert.ok(r.getByText(/Showing 1 request; more may be pending/));
  assert.ok(failList);
  failList(new Error('History unavailable'));
  await ui.waitFor(() => assert.ok(r.getByText('Approve')));
  ui.fireEvent.click(r.getByText('Refresh'));
  await ui.waitFor(() => assert.ok(r.getByText('visitor2')));
  failList(new Error('History unavailable'));
  await ui.waitFor(() => assert.ok(r.getByText('History unavailable')));
  assert.equal(r.queryByText('visitor1'), null);
});

for (const first of ['inbox', 'list'] as const) {
  test(`Requests waits for each read independently when ${first} finishes first`, async () => {
    const { MembershipRequests } =
      await import('../src/components/membership-requests');
    const { mockBridge } = await import('../src/mock/mock-bridge');
    const { FIXTURE } = await import('../src/mock/fixture');
    const team = FIXTURE.stores.find(
      (store): store is TeamStore => store.kind === 'team',
    );
    assert.ok(team);
    const reads = new Map<
      string,
      {
        resolve: (reply: InvitationReply) => void;
        reject: (error: Error) => void;
      }
    >();
    const bridge = {
      ...mockBridge(FIXTURE),
      invitation: (
        _p: string,
        _a: string,
        action: InvitationAction,
      ): Promise<InvitationReply> =>
        new Promise((resolve, reject) =>
          reads.set(action.action, { resolve, reject }),
        ),
    };
    const r = ui.render(
      await overlay(
        createElement(MembershipRequests, {
          bridge,
          team,
          servers: FIXTURE.servers,
          serverLabel: 'Example',
          onComplete() {},
          onInvite() {},
          onActivity() {},
        }),
      ),
    );
    assert.ok(r.getByText('Loading requests…'));
    assert.ok(r.getByText('Loading invitations…'));
    assert.equal(r.queryByText('No requests yet.'), null);
    assert.equal(r.queryByText('No in-progress invitations.'), null);
    await ui.act(async () => reads.get(first)!.resolve([]));
    assert.ok(
      r.getByText(
        first === 'inbox' ? 'No requests yet.' : 'No in-progress invitations.',
      ),
    );
    assert.ok(
      r.getByText(
        first === 'inbox' ? 'Loading invitations…' : 'Loading requests…',
      ),
    );
    const second = first === 'inbox' ? 'list' : 'inbox';
    await ui.act(async () =>
      reads.get(second)!.reject(new Error('Read failed')),
    );
    assert.ok(r.getByText('Read failed'));
    assert.equal(
      r.queryByText(
        second === 'inbox' ? 'No requests yet.' : 'No in-progress invitations.',
      ),
      null,
    );
    ui.fireEvent.click(r.getByText('Refresh'));
    await ui.act(async () => {
      reads.get('inbox')!.resolve({
        rows: [
          {
            request_id: 'a'.repeat(32),
            username: 'waiting-user',
            verified: true,
          },
        ],
      });
      reads.get('list')!.resolve([]);
    });
    assert.ok(r.getByText('waiting-user'));
    ui.fireEvent.click(r.getByText('Refresh'));
    assert.ok(r.getByText('waiting-user'));
    assert.ok(r.getByText('Loading requests…'));
    await ui.act(async () => {
      reads.get('inbox')!.resolve([]);
      reads.get('list')!.resolve([]);
    });
    assert.equal(r.queryByText('waiting-user'), null);
    assert.ok(r.getByText('No requests yet.'));
  });
}

test('activity opens with loading, keeps known membership work, and recovers from a failed read', async () => {
  const { InvitationActivitySheet } =
    await import('../src/components/invitation-activity-sheet');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const { FIXTURE } = await import('../src/mock/fixture');
  const team = FIXTURE.stores.find(
    (store): store is TeamStore => store.kind === 'team',
  );
  assert.ok(team);
  let fail!: (error: Error) => void;
  let resolve!: (reply: InvitationReply) => void;
  const bridge = {
    ...mockBridge(FIXTURE),
    invitation: (_p: string, _a: string, action: InvitationAction) =>
      action.action === 'list'
        ? new Promise<InvitationReply>((done, reject) => {
            resolve = done;
            fail = reject;
          })
        : Promise.resolve([]),
  };
  const r = ui.render(
    await overlay(
      createElement(InvitationActivitySheet, {
        bridge,
        team,
        onClose() {},
        membership: {
          operations: [
            {
              kind: 'team-member-addition',
              alias: team.alias,
              target: 'existing-user',
            },
          ],
          onResume() {},
        },
      }),
    ),
  );
  assert.ok(r.getByText('Loading invitation activity…'));
  assert.ok(r.getByText('Adding existing-user'));
  assert.equal((r.getByText('Close') as HTMLButtonElement).disabled, false);
  assert.equal(r.queryByText('Nothing is waiting on you.'), null);
  await ui.act(async () => fail(new Error('Activity failed')));
  assert.ok(r.getByText('Activity failed'));
  assert.equal(r.queryByText('Loading invitation activity…'), null);
  ui.fireEvent.click(r.getByText('Refresh'));
  await ui.act(async () => resolve([]));
  assert.equal(r.queryByText('Activity failed'), null);
  assert.ok(r.getByText('Adding existing-user'));
});
