import { renameStates } from './rename-contract';
export type BotAction =
  | { action: 'list' | 'load-file' | 'unload' }
  | {
      action: 'prepare';
      role: 'owner' | 'admin' | { member: { visibility: number } };
      pin: string | null;
    }
  | { action: 'attempt' | 'status'; operation_id: string; pin: string | null }
  | { action: 'cancel' | 'export-file'; operation_id: string }
  | { action: 'revoke'; device_id: string; pin: string | null };
export interface BotEnrollment {
  operation_id: string;
  account_alias: string;
  name: string;
  device_id: string;
  role: string;
  state: string;
  hardware_required: boolean;
  export_available: boolean;
}
export interface BotReply {
  rows: BotEnrollment[];
  message: string;
}
export function decodeBotReply(value: unknown): BotReply {
  if (!value || typeof value !== 'object')
    throw new Error('Invalid bot response');
  const v = value as Record<string, unknown>;
  if (
    Object.keys(v).length !== 2 ||
    typeof v.message !== 'string' ||
    v.message.length > 256 ||
    !Array.isArray(v.rows) ||
    v.rows.length > 160
  )
    throw new Error('Invalid bot response');
  const keys = [
    'operation_id',
    'account_alias',
    'name',
    'device_id',
    'role',
    'state',
    'hardware_required',
    'export_available',
  ];
  for (const row of v.rows as unknown[]) {
    if (!row || typeof row !== 'object')
      throw new Error('Invalid bot enrollment');
    const p = row as Record<string, unknown>;
    if (
      Object.keys(p).length !== keys.length ||
      Object.keys(p).some((k) => !keys.includes(k)) ||
      typeof p.operation_id !== 'string' ||
      !/^[0-9a-f]{32}$/.test(p.operation_id) ||
      typeof p.account_alias !== 'string' ||
      !/^[a-zA-Z0-9_-]{1,64}$/.test(p.account_alias) ||
      typeof p.device_id !== 'string' ||
      !/^13[0-9a-f]{64}$/.test(p.device_id) ||
      typeof p.name !== 'string' ||
      !/^[a-zA-Z0-9]{5}$/.test(p.name) ||
      typeof p.role !== 'string' ||
      p.role.length > 64 ||
      typeof p.state !== 'string' ||
      !(renameStates as readonly string[]).includes(p.state) ||
      typeof p.hardware_required !== 'boolean' ||
      typeof p.export_available !== 'boolean'
    )
      throw new Error('Invalid bot enrollment');
  }
  return value as BotReply;
}
