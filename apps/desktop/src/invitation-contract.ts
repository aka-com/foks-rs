export type InvitationRole =
  'admin' | 'owner' | { member: { visibility: number } };
export type InvitationAction =
  | { action: 'preview' | 'accept'; invite: string }
  | {
      action: 'preview-remote' | 'accept-remote';
      remote_profile: string;
      invite: string;
    }
  | {
      action: 'accept-team';
      invite: string;
      source_team_alias: string;
      source_role: InvitationRole;
    }
  | {
      action: 'accept-team-remote';
      remote_profile: string;
      invite: string;
      source_team_alias: string;
      source_role: InvitationRole;
    }
  | {
      action: 'create' | 'inbox' | 'inbox-count' | 'pending-approvals';
      team_alias: string;
    }
  | { action: 'range'; team_alias: string; raise: boolean }
  | { action: 'attempt' | 'status' | 'cancel'; operation_id: string }
  | {
      action: 'attempt-remote' | 'status-remote';
      remote_profile: string;
      operation_id: string;
    }
  | { action: 'list' }
  | {
      action: 'approve';
      team_alias: string;
      request_id: string;
      role: InvitationRole;
    }
  | { action: 'reject'; team_alias: string; request_id: string }
  | {
      action: 'inspect-remote';
      remote_profile: string;
      team_alias: string;
      request_id: string;
    }
  | {
      action: 'approve-remote';
      remote_profile: string;
      team_alias: string;
      request_id: string;
      role: InvitationRole;
    }
  | {
      action: 'sync-remote';
      remote_profile: string;
      team_id: string;
      source_team_alias?: string;
      source_role?: InvitationRole;
    };
export interface InvitationRow {
  operation_id?: string;
  request_id?: string;
  team_id?: string;
  host_id?: string;
  joiner_id?: string;
  state?: string;
  invite?: string;
  name?: string;
  username?: string;
  error?: string;
  joiner_kind?: string;
  remote_profile?: string;
  source_profile?: string;
  role?: { kind: number; visibility: number };
  source_role?: { kind: number; visibility: number };
  time?: number;
  team_sequence?: number;
  key_generations?: number;
  /** The number of pending rows, from the count-only inbox action. */
  count?: number;
  verified?: boolean;
  membership?: boolean;
  membership_verified?: boolean;
  delivery_acknowledged?: boolean;
  hardware_required?: boolean;
  remote?: boolean;
  possibly_truncated?: boolean;
  rows?: InvitationRow[];
}
export type InvitationReply = InvitationRow | InvitationRow[];
export function decodeInvitationReply(value: unknown): InvitationReply {
  const strings = new Set([
    'operation_id',
    'request_id',
    'team_id',
    'host_id',
    'joiner_id',
    'state',
    'invite',
    'name',
    'username',
    'error',
    'joiner_kind',
    'remote_profile',
    'source_profile',
  ]);
  const numbers = new Set([
    'time',
    'team_sequence',
    'key_generations',
    'count',
  ]);
  const booleans = new Set([
    'verified',
    'membership',
    'membership_verified',
    'delivery_acknowledged',
    'hardware_required',
    'remote',
    'possibly_truncated',
  ]);
  const fail = (): never => {
    throw new Error('Invalid invitation response.');
  };
  const row = (v: unknown, depth = 0): InvitationRow => {
    if (!v || typeof v !== 'object' || Array.isArray(v) || depth > 1)
      return fail();
    for (const [key, x] of Object.entries(v as Record<string, unknown>)) {
      if (strings.has(key)) {
        if (typeof x !== 'string' || x.length > 1024) return fail();
        if (
          ['operation_id', 'request_id'].includes(key) &&
          !/^[a-f0-9]{32}$/.test(x)
        )
          return fail();
        if (
          ['team_id', 'host_id', 'joiner_id'].includes(key) &&
          !/^[a-f0-9]{66}$/.test(x)
        )
          return fail();
      } else if (numbers.has(key)) {
        if (typeof x !== 'number' || !Number.isSafeInteger(x) || x < 0)
          return fail();
      } else if (booleans.has(key)) {
        if (typeof x !== 'boolean') return fail();
      } else if (key === 'rows') {
        if (!Array.isArray(x) || x.length > 2000) return fail();
        x.forEach((r) => row(r, depth + 1));
      } else if (key === 'role' || key === 'source_role') {
        if (
          !x ||
          typeof x !== 'object' ||
          Object.keys(x).some((k) => !['kind', 'visibility'].includes(k))
        )
          return fail();
        const r = x as { kind: number; visibility: number };
        if (
          ![1, 2, 3].includes(r.kind) ||
          !Number.isInteger(r.visibility) ||
          r.visibility < -32768 ||
          r.visibility > 32767
        )
          return fail();
      } else return fail();
    }
    return v;
  };
  if (Array.isArray(value)) {
    if (value.length > 2000) return fail();
    return value.map((v) => row(v));
  }
  return row(value);
}
