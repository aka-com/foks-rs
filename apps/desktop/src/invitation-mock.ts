/** Local-only invitation fixtures for browser journeys, not a protocol emulator. */
import type { Bridge } from './bridge';
import type { InvitationRow } from './invitation-contract';
import type { Store } from './model';

export function mockInvitations(
  stores: readonly Store[],
): Bridge['invitation'] {
  let serial = 0;
  const operations = new Map<string, { owner: string; row: InvitationRow }>();
  const requests = new Map<string, InvitationRow[]>();
  return async (profile, account, action) => {
    const owner = `${profile}/${account}`;
    const team =
      'team_alias' in action
        ? stores.find(
            (store) =>
              store.kind === 'team' &&
              store.server === profile &&
              store.account === account &&
              store.alias === action.team_alias,
          )
        : undefined;
    if ('team_alias' in action && !team)
      throw new Error('Fixture team not found for this account.');
    const key = `${owner}/${team?.id}`;
    switch (action.action) {
      case 'create': {
        const id = (++serial).toString(16).padStart(32, '0');
        const row = {
          operation_id: id,
          state: 'prepared',
          team_id: team?.kind === 'team' ? team.team_id_hex : undefined,
          invite: `Fixture invitation for ${key}`,
        };
        operations.set(id, { owner, row });
        return { operation_id: id, state: 'prepared' };
      }
      case 'attempt':
      case 'status': {
        const operation = operations.get(action.operation_id);
        if (!operation || operation.owner !== owner)
          throw new Error('Fixture operation not found.');
        if (action.action === 'attempt') operation.row.state = 'complete';
        return { ...operation.row };
      }
      case 'list':
        return {
          rows: [...operations.values()]
            .filter((op) => op.owner === owner)
            .map((op) => ({ ...op.row })),
        };
      case 'inbox':
      case 'inbox-count':
        if (!requests.has(key))
          requests.set(key, [
            {
              request_id: 'a'.repeat(32),
              username: 'fixture-joiner',
              verified: true,
            },
            {
              request_id: 'b'.repeat(32),
              username: 'fixture-second-joiner',
              verified: true,
            },
          ]);
        return action.action === 'inbox-count'
          ? { count: requests.get(key)!.length }
          : { rows: requests.get(key)!.map((row) => ({ ...row })) };
      case 'approve':
      case 'reject':
        requests.set(
          key,
          (requests.get(key) ?? []).filter(
            (row) => row.request_id !== action.request_id,
          ),
        );
        return { state: 'complete' };
      case 'pending-approvals':
        return { rows: [] };
      // The join journey needs a team to confirm before it can prepare a
      // request; the fixture resolves any invitation to the same one.
      case 'preview':
      case 'preview-remote':
        return {
          team_id: `01${'3f9c'.repeat(16)}`,
          host_id: `01${'8e02'.repeat(16)}`,
          name: 'Platform Engineering',
          membership: false,
        };
      case 'accept':
      case 'accept-remote':
      case 'accept-team':
      case 'accept-team-remote': {
        const id = (++serial).toString(16).padStart(32, '0');
        const row = {
          operation_id: id,
          state: 'prepared',
          remote: action.action.endsWith('-remote'),
        };
        operations.set(id, { owner, row });
        return { ...row };
      }
      default:
        throw new Error(
          'This invitation operation requires a connected agent.',
        );
    }
  };
}
