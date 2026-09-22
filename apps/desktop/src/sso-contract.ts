export type SsoPurpose = 'signup' | 'link-existing' | 'reauthenticate';
export interface SsoAccountStatus {
  state:
    | 'device-only'
    | 'migration-eligible'
    | 'linked'
    | 'locked-out'
    | 'not-eligible';
  rolloutMode: number;
  providerBlockedReason: number;
  issuer: string;
  authorizationEpoch: number;
  authorizationGeneration: number;
}
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
  | { action: 'begin'; purpose: SsoPurpose; pin: string | null }
  | { action: 'account-status'; pin: string | null }
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
  'device-only',
  'link-needed',
  'locked-out',
  'linked',
  'not-eligible',
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
  operationId: string | null;
  accountAlias: string;
  purpose: SsoPurpose;
  accountStatus: SsoAccountStatus | null;
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
    'purpose',
    'accountStatus',
    'state',
    'browserAvailable',
    'expiresAtMs',
    'serviceAccess',
  ];
  if (
    Object.keys(v).some((k) => !keys.includes(k)) ||
    Object.keys(v).length !== keys.length ||
    (v.operationId !== null &&
      (typeof v.operationId !== 'string' ||
        !/^[a-f0-9]{32}$/.test(v.operationId))) ||
    (v.operationId === null) !== (v.accountStatus !== null) ||
    (v.operationId === null && v.browserAvailable !== false) ||
    typeof v.accountAlias !== 'string' ||
    !/^[a-zA-Z0-9_-]{1,64}$/.test(v.accountAlias) ||
    !['signup', 'link-existing', 'reauthenticate'].includes(
      String(v.purpose),
    ) ||
    !validAccountStatus(v.accountStatus) ||
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

function validAccountStatus(value: unknown): boolean {
  if (value === null) return true;
  if (!value || typeof value !== 'object') return false;
  const v = value as Record<string, unknown>;
  const keys = [
    'state',
    'rolloutMode',
    'providerBlockedReason',
    'issuer',
    'authorizationEpoch',
    'authorizationGeneration',
  ];
  return (
    Object.keys(v).length === keys.length &&
    Object.keys(v).every((k) => keys.includes(k)) &&
    [
      'device-only',
      'migration-eligible',
      'linked',
      'locked-out',
      'not-eligible',
    ].includes(String(v.state)) &&
    Number.isInteger(v.rolloutMode) &&
    Number(v.rolloutMode) >= 0 &&
    Number(v.rolloutMode) <= 2 &&
    Number.isInteger(v.providerBlockedReason) &&
    Number(v.providerBlockedReason) >= 0 &&
    Number(v.providerBlockedReason) <= 4 &&
    typeof v.issuer === 'string' &&
    v.issuer.length <= 4096 &&
    Number.isSafeInteger(v.authorizationEpoch) &&
    Number(v.authorizationEpoch) >= 0 &&
    Number.isSafeInteger(v.authorizationGeneration) &&
    Number(v.authorizationGeneration) >= 0
  );
}
