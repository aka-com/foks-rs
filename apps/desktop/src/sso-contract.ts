export type SsoAction =
  | {
      action: 'begin-yubi-signup';
      card_serial: number;
      signing_slot: number;
      pq_slot: number;
      pin: string;
      device_name: string;
      invite: string;
    }
  | { action: 'finish-yubi-signup'; operation_id: string; pin: string }
  | { action: 'begin'; for_login: boolean }
  | { action: 'status' | 'poll' | 'cancel'; operation_id: string }
  | { action: 'finish-login'; operation_id: string; pin: string | null }
  | {
      action: 'finish-signup';
      operation_id: string;
      device_name: string;
      invite: string;
      passphrase: string | null;
    };
export const ssoStates = [
  'prepared',
  'waiting',
  'ready',
  'submitting',
  'complete',
  'cancelled',
  'expired',
  'submission-unknown',
  'rejected',
  'denied',
  'provider-unavailable',
  'reauthentication-required',
  'hardware-verification-required',
  'service-unavailable',
] as const;
export interface SsoProgress {
  operationId: string;
  accountAlias: string;
  forLogin: boolean;
  state: (typeof ssoStates)[number];
  browserAvailable: boolean;
  expiresAtMs: number;
  serviceAccess: boolean;
}
export function decodeSsoProgress(value: unknown): SsoProgress {
  if (!value || typeof value !== 'object')
    throw new Error('Invalid authentication progress');
  const v = value as Record<string, unknown>;
  const keys = [
    'operationId',
    'accountAlias',
    'forLogin',
    'state',
    'browserAvailable',
    'expiresAtMs',
    'serviceAccess',
  ];
  if (
    Object.keys(v).some((k) => !keys.includes(k)) ||
    Object.keys(v).length !== keys.length ||
    typeof v.operationId !== 'string' ||
    !/^[a-f0-9]{32}$/.test(v.operationId) ||
    typeof v.accountAlias !== 'string' ||
    !/^[a-zA-Z0-9_-]{1,64}$/.test(v.accountAlias) ||
    typeof v.forLogin !== 'boolean' ||
    typeof v.state !== 'string' ||
    !(ssoStates as readonly string[]).includes(v.state) ||
    typeof v.browserAvailable !== 'boolean' ||
    typeof v.serviceAccess !== 'boolean' ||
    !Number.isSafeInteger(v.expiresAtMs) ||
    (v.expiresAtMs as number) < 0
  )
    throw new Error('Invalid authentication progress');
  return v as unknown as SsoProgress;
}
