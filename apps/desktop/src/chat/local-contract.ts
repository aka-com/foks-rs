import { decodeChatScope } from '../chat-contract';
import type { ChatScope } from '../chat-contract';
export interface LocalSettings {
  enabled: boolean;
  previews: boolean;
  overrides: Record<string, boolean>;
}
export interface LocalSession {
  migrationRemaining?: number;
  epoch: string;
  available: boolean;
  settings: LocalSettings;
  activation?: { storeId: string; scope: ChatScope; channel: string };
}
export type LocalAction =
  | { action: 'recover-intents' }
  | { action: 'begin' }
  | { action: 'take-activation' }
  | { action: 'clear'; epoch: string }
  | { action: 'end'; epoch: string }
  | {
      action: 'configure';
      enabled?: boolean;
      previews?: boolean;
      storeId?: string;
      scope?: ChatScope;
      channel?: string;
      mode?: boolean | null;
    }
  | {
      action: 'display';
      epoch: string;
      storeId: string;
      scope: ChatScope;
      channel: string;
      body?: string;
      count: number;
      incomplete: boolean;
    };
export function decodeLocalSession(value: unknown): LocalSession {
  const bad = () => {
    throw new Error('Invalid local notification response.');
  };
  if (!value || typeof value !== 'object') return bad();
  const v = value as Record<string, unknown>;
  if (
    typeof v.epoch !== 'string' ||
    !/^[0-9a-f]{32}$/.test(v.epoch) ||
    typeof v.available !== 'boolean' ||
    !v.settings ||
    typeof v.settings !== 'object'
  )
    return bad();
  const s = v.settings as Record<string, unknown>;
  if (
    typeof s.enabled !== 'boolean' ||
    typeof s.previews !== 'boolean' ||
    !s.overrides ||
    typeof s.overrides !== 'object' ||
    Array.isArray(s.overrides)
  )
    return bad();
  const entries = Object.entries(s.overrides);
  if (
    entries.length > 4096 ||
    entries.some(
      ([k, v]) =>
        !/^[0-9a-f]{64}\/[0-9a-f]{64}$/.test(k) || typeof v !== 'boolean',
    )
  )
    return bad();
  let activation: LocalSession['activation'];
  if (v.activation !== undefined && v.activation !== null) {
    if (typeof v.activation !== 'object') return bad();
    const a = v.activation as Record<string, unknown>;
    if (
      typeof a.storeId !== 'string' ||
      a.storeId.length > 4096 ||
      typeof a.channel !== 'string' ||
      !/^[0-9a-f]{32}$/.test(a.channel)
    )
      return bad();
    activation = {
      storeId: a.storeId,
      scope: decodeChatScope(a.scope, a.storeId),
      channel: a.channel,
    };
  }
  if (
    v.migrationRemaining !== undefined &&
    (!Number.isInteger(v.migrationRemaining) ||
      (v.migrationRemaining as number) < 0 ||
      (v.migrationRemaining as number) > 128)
  )
    return bad();
  return {
    ...(v.migrationRemaining !== undefined
      ? { migrationRemaining: v.migrationRemaining as number }
      : {}),
    ...(activation ? { activation } : {}),
    epoch: v.epoch,
    available: v.available,
    settings: {
      enabled: s.enabled,
      previews: s.previews,
      overrides: Object.fromEntries(entries),
    },
  };
}
export async function notificationKey(
  scope: ChatScope,
  channel: string,
): Promise<string> {
  const hash = async (parts: string[]) =>
    Array.from(
      new Uint8Array(
        await crypto.subtle.digest(
          'SHA-256',
          new TextEncoder().encode(JSON.stringify(parts)),
        ),
      ),
    )
      .map((b) => b.toString(16).padStart(2, '0'))
      .join('');
  return `${await hash([scope.store.profile])}/${await hash([scope.host, scope.actor, scope.store.team_id, channel])}`;
}
