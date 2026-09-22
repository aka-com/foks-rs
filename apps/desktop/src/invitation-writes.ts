import { normalizeCommandError } from './bridge';
import { INVITATION_ACTIVITY } from './invitation-activity';
import type { InvitationRole, InvitationRow } from './invitation-contract';
import type { InvitationGrantRole } from './invitation-labels';
import type { MetadataRepository } from './metadata-repository';
import { pendingOperationKey } from './operation-queries';

export function settleInvitationWrite(
  queries: MetadataRepository,
  profile: string,
  account: string,
): void {
  queries.invalidate(['invitation-recovery', profile, account]);
  queries.invalidate(['team-requests', profile, account]);
  queries.invalidate(pendingOperationKey(profile));
  window.dispatchEvent(
    new window.CustomEvent(INVITATION_ACTIVITY, {
      detail: { profile, account },
    }),
  );
}

export function nativeRole(value: InvitationGrantRole): InvitationRole {
  return value === 'member' ? { member: { visibility: 0 } } : value;
}

export function rowRole(role: {
  kind: number;
  visibility: number;
}): InvitationRole {
  return role.kind === 1
    ? { member: { visibility: role.visibility } }
    : role.kind === 2
      ? 'admin'
      : 'owner';
}

export function grantRoleName(role: InvitationGrantRole): string {
  return role === 'member' ? 'Member' : role === 'admin' ? 'Admin' : 'Owner';
}

export function hardwareUnlockRequested(error: unknown): boolean {
  const typed = normalizeCommandError(error);
  return (
    typed.code === 'hardware-required' ||
    /unlock the account key|security key|hardware key/i.test(typed.message)
  );
}

export function isNestingRefusal(error: unknown): boolean {
  return /index range/i.test(normalizeCommandError(error).message);
}

export function invitationRows(
  reply: InvitationRow | InvitationRow[],
): InvitationRow[] {
  return Array.isArray(reply) ? reply : (reply.rows ?? [reply]);
}

export function invitationRow(
  reply: InvitationRow | InvitationRow[],
): InvitationRow {
  return Array.isArray(reply) ? (reply[0] ?? {}) : reply;
}

export function unfinishedOperation(row: InvitationRow): boolean {
  return (
    Boolean(row.operation_id) &&
    !row.request_id &&
    row.state !== 'complete' &&
    row.state !== 'cancelled'
  );
}

export function operationStateText(state: string | undefined): string {
  switch (state) {
    case 'prepared':
      return 'Creation interrupted';
    case 'submitting':
      return 'Submitting';
    case 'submission-unknown':
      return 'Submitted, no response from the server';
    case 'acknowledged':
      return 'Acknowledged by the server';
    case 'complete':
      return 'Ready';
    case 'cancelled':
      return 'Discarded';
    default:
      return state ?? 'Unknown';
  }
}

export function invitationInstructions(input: {
  team: string;
  server: string;
  invite: string;
  approver?: string;
}): string {
  const lines = [
    `You're invited to the team ${input.team} on FOKS.`,
    '',
    '1. Install FOKS and choose "Join an existing team".',
    `2. Server: ${input.server}`,
    '3. Create your account, then paste this invitation:',
    `   ${input.invite}`,
    '',
    input.approver
      ? `Your request will be approved by ${input.approver}.`
      : 'Your request will be approved by a team administrator.',
  ];
  return lines.join('\n');
}
