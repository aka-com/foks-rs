export type RenameAction =
  | { action: 'prepare'; username: string; pin: string | null }
  | { action: 'attempt' | 'status'; operation_id: string; pin: string | null }
  | { action: 'cancel'; operation_id: string };
export const renameStates = [
  'prepared',
  'submitting',
  'submission-unknown',
  'remote-verified',
  'complete',
  'rejected',
] as const;
export interface RenameProgress {
  operation_id: string;
  account_alias: string;
  state: (typeof renameStates)[number];
  target: string | null;
  current_username: string | null;
  hardware_required: boolean;
}
export function decodeRenameProgress(value: unknown): RenameProgress[] {
  if (!Array.isArray(value) || value.length > 160)
    throw new Error('Invalid rename response');
  for (const p of value) {
    if (!p || typeof p !== 'object') throw new Error('Invalid rename progress');
    const v = p as Record<string, unknown>;
    const keys = [
      'operation_id',
      'account_alias',
      'state',
      'target',
      'current_username',
      'hardware_required',
    ];
    if (
      Object.keys(v).length !== keys.length ||
      Object.keys(v).some((k) => !keys.includes(k)) ||
      typeof v.operation_id !== 'string' ||
      !/^[a-f0-9]{32}$/.test(v.operation_id) ||
      typeof v.account_alias !== 'string' ||
      !/^[a-zA-Z0-9_-]{1,64}$/.test(v.account_alias) ||
      typeof v.state !== 'string' ||
      !(renameStates as readonly string[]).includes(v.state) ||
      (v.target !== null &&
        (typeof v.target !== 'string' || v.target.length > 256)) ||
      (v.current_username !== null &&
        (typeof v.current_username !== 'string' ||
          v.current_username.length > 4096)) ||
      typeof v.hardware_required !== 'boolean'
    )
      throw new Error('Invalid rename progress');
  }
  return value as RenameProgress[];
}
