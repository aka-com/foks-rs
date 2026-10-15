import { decodeChatScope } from '../chat-contract';
import type { ChatScope } from '../chat-contract';
import { sha256Hex } from './sha256';
/**
 * Which new messages raise a desktop alert, as the native settings store it:
 * `true` every message, `false` none, `'mentions'` the messages whose text
 * contains `@` and the reader's username.
 */
export type LocalAlertMode = boolean | 'mentions';
/** The most entries each keyed setting holds, as the native settings bound it. */
export const LOCAL_KEYED_LIMIT = 4096;
/** The most characters an alert's title holds, as `chat_local.rs` bounds it. */
export const ALERT_TITLE_CHARS = 128;
/** The most characters of its own name a notification service reports. */
const SERVICE_NAME_CHARS = 64;
/** Which system shows desktop alerts, as the native session reports it. */
export type AlertBackend = 'macos' | 'freedesktop' | 'none';
export interface LocalSettings {
  enabled: boolean;
  previews: boolean;
  /**
   * Whether the app icon shows the unread count. The native settings write
   * it only when off, so absent means on.
   */
  badge?: boolean;
  /** A channel's own alert mode, by {@link notificationKey}. */
  overrides: Record<string, LocalAlertMode>;
  /**
   * The channels and teams muted on this device, by {@link notificationKey}
   * or {@link teamKey}. A team's mute covers each of its channels.
   */
  muted: readonly string[];
  /** A team's default alert mode, by {@link teamKey}. */
  teams: Record<string, LocalAlertMode>;
}
export interface LocalSession {
  migrationRemaining?: number;
  epoch: string;
  available: boolean;
  /**
   * Which system shows desktop alerts: macOS Notification Center, a
   * freedesktop.org notification service (Linux), or none.
   */
  backend?: AlertBackend;
  /** The notification service's own name, such as "GNOME Shell". */
  service?: string;
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
      badge?: boolean;
      storeId?: string;
      scope?: ChatScope;
      /**
       * The channel to configure; without it, `mode` is the team default and
       * `muted` mutes the team.
       */
      channel?: string;
      /** `null` removes the channel's own mode or the team default. */
      mode?: LocalAlertMode | null;
      muted?: boolean;
    }
  | {
      action: 'display';
      epoch: string;
      storeId: string;
      scope: ChatScope;
      channel: string;
      /**
       * The channel and team, shown as the alert's title while previews are
       * on; at most {@link ALERT_TITLE_CHARS} characters, on one line.
       */
      title?: string;
      body?: string;
      count: number;
      incomplete: boolean;
      /**
       * The reader has read every message the channel's earlier alerts stood
       * for, so a platform that counts a channel's alerts in one entry starts
       * its count again.
       */
      restart?: boolean;
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
  const key = /^[0-9a-f]{64}\/[0-9a-f]{64}$/;
  const modes = (value: unknown): [string, LocalAlertMode][] | undefined => {
    if (!value || typeof value !== 'object' || Array.isArray(value)) return;
    const entries = Object.entries(value);
    if (
      entries.length > LOCAL_KEYED_LIMIT ||
      entries.some(
        ([k, mode]) =>
          !key.test(k) || (typeof mode !== 'boolean' && mode !== 'mentions'),
      )
    )
      return;
    return entries as [string, LocalAlertMode][];
  };
  // The native settings leave out an empty muted set and team defaults.
  const overrides = modes(s.overrides),
    teams = s.teams === undefined ? [] : modes(s.teams),
    muted = s.muted === undefined ? [] : s.muted;
  if (
    typeof s.enabled !== 'boolean' ||
    typeof s.previews !== 'boolean' ||
    (s.badge !== undefined && typeof s.badge !== 'boolean') ||
    (v.backend !== undefined &&
      v.backend !== 'macos' &&
      v.backend !== 'freedesktop' &&
      v.backend !== 'none') ||
    (v.service !== undefined &&
      (typeof v.service !== 'string' ||
        !v.service ||
        Array.from(v.service).length > SERVICE_NAME_CHARS)) ||
    !overrides ||
    !teams ||
    !Array.isArray(muted) ||
    muted.length > LOCAL_KEYED_LIMIT ||
    muted.some((k) => typeof k !== 'string' || !key.test(k)) ||
    new Set(muted).size !== muted.length
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
    ...(v.backend !== undefined ? { backend: v.backend } : {}),
    ...(v.service !== undefined ? { service: v.service } : {}),
    settings: {
      enabled: s.enabled,
      previews: s.previews,
      ...(s.badge !== undefined ? { badge: s.badge } : {}),
      overrides: Object.fromEntries(overrides),
      muted: muted as string[],
      teams: Object.fromEntries(teams),
    },
  };
}
const hash = (parts: string[]) =>
  sha256Hex(new TextEncoder().encode(JSON.stringify(parts)));
// Derived keys by scope object, so a projection that applies mutes on every
// publication hashes each channel once per synchronized scope; a scope no
// snapshot holds any longer takes its keys with it. Scopes are frozen DTOs.
const channelKeys = new WeakMap<ChatScope, Map<string, string>>();
const teamKeys = new WeakMap<ChatScope, string>();
/**
 * A channel's key in the local settings: digests of the profile and of the
 * host, actor, team and channel ids, as `chat_local.rs` derives it. The
 * settings file holds no alias, name or id in the clear.
 */
export function notificationKey(scope: ChatScope, channel: string): string {
  let derived = channelKeys.get(scope);
  if (!derived) {
    derived = new Map();
    channelKeys.set(scope, derived);
  }
  let key = derived.get(channel);
  if (key === undefined) {
    key = `${hash([scope.store.profile])}/${hash([scope.host, scope.actor, scope.store.team_id, channel])}`;
    derived.set(channel, key);
  }
  return key;
}
/** A team's key in the local settings, as `chat_local.rs` derives it. */
export function teamKey(scope: ChatScope): string {
  let key = teamKeys.get(scope);
  if (key === undefined) {
    key = `${hash([scope.store.profile])}/${hash([scope.host, scope.actor, scope.store.team_id])}`;
    teamKeys.set(scope, key);
  }
  return key;
}
/** Whether a team is muted on this device; see {@link LocalSettings.muted}. */
export function teamMuted(settings: LocalSettings, scope: ChatScope): boolean {
  return settings.muted.includes(teamKey(scope));
}
/** Why desktop alerts are unavailable, as the shell states it. */
export function alertsUnavailableReason(session: LocalSession): string {
  return session.backend === 'freedesktop'
    ? 'No desktop notification service was found.'
    : 'Desktop alerts are unavailable in this build.';
}
/** A channel's or team's alert mode in the terms the shell offers. */
export type AlertChoice = 'all' | 'mentions' | 'none';
export function alertChoice(mode: LocalAlertMode): AlertChoice {
  return mode === 'mentions' ? 'mentions' : mode ? 'all' : 'none';
}
export function storedAlertMode(choice: AlertChoice): LocalAlertMode {
  return choice === 'mentions' ? 'mentions' : choice === 'all';
}
/** What decided a channel's alert mode; see {@link channelAlerts}. */
export type AlertSource = 'channel' | 'muted' | 'team' | 'device';
/**
 * The alert mode a channel without its own takes: none while it is muted,
 * else its team's default, else every message. `muted` is the projection's
 * flag, which includes a mute on this device.
 */
export function inheritedAlerts(
  settings: LocalSettings,
  scope: ChatScope,
  muted: boolean,
): { mode: AlertChoice; from: AlertSource } {
  if (muted) return { mode: 'none', from: 'muted' };
  const team = settings.teams[teamKey(scope)];
  if (team !== undefined) return { mode: alertChoice(team), from: 'team' };
  return { mode: 'all', from: 'device' };
}
/**
 * The alert mode that applies to a channel and what decided it: the
 * channel's own mode, else {@link inheritedAlerts}. `chat_local.rs` refuses
 * a desktop alert by the same order.
 */
export function channelAlerts(
  settings: LocalSettings,
  scope: ChatScope,
  channel: string,
  muted: boolean,
): { mode: AlertChoice; from: AlertSource } {
  const own = settings.overrides[notificationKey(scope, channel)];
  return own === undefined
    ? inheritedAlerts(settings, scope, muted)
    : { mode: alertChoice(own), from: 'channel' };
}
