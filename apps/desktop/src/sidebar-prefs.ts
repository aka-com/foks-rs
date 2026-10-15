/**
 * The persisted display preferences of the rail and the Chat column.
 *
 * Each is one registered `localStorage` record. `sideCollapsed` and
 * `chatPreviews` are booleans encoded as `'1'` / `'0'`; `chatChannelSort`
 * names the column's channel order; `chatFoldedTeams` is a JSON array of
 * hashed team ids. Storage is unavailable in restricted contexts, and a
 * display preference is never worth failing the shell for: an unreadable
 * value reads as its default and an unwritable one lasts for this session
 * only.
 */

import { hashId } from './diagnostics/log';
import {
  readStorage,
  STORAGE_KEYS,
  type StorageKey,
  writeStorage,
} from './platform/device-storage';

function read(key: StorageKey, fallback = false): boolean {
  const value = readStorage(key);
  return value === null ? fallback : value === '1';
}

function write(key: StorageKey, value: boolean): void {
  writeStorage(key, value ? '1' : '0');
}

/** The stored collapse preference. False (expanded) when unset or unreadable. */
export function storedSideCollapsedPref(): boolean {
  return readStorage(STORAGE_KEYS.sideCollapsed) === '1';
}

/** Persists the collapse preference. */
export function rememberSideCollapsed(collapsed: boolean): void {
  // Storage may be unavailable; the preference then lasts for this session only.
  writeStorage(STORAGE_KEYS.sideCollapsed, collapsed ? '1' : '0');
}

/**
 * The order of each team's channels in the Chat column: the general channel
 * first and then A to Z, or the channel with the newest message first.
 */
export type ChannelSort = 'alphabetical' | 'recent';

/** The stored channel order. Alphabetical when unset or unreadable. */
export function storedChannelSort(): ChannelSort {
  return readStorage(STORAGE_KEYS.chatChannelSort) === 'recent'
    ? 'recent'
    : 'alphabetical';
}

export function rememberChannelSort(sort: ChannelSort): void {
  writeStorage(STORAGE_KEYS.chatChannelSort, sort);
}

/**
 * Whether a channel row carries its last message and that message's time
 * under its name. On when unset or unreadable.
 */
export function storedChannelPreviews(): boolean {
  return read(STORAGE_KEYS.chatPreviews, true);
}

export function rememberChannelPreviews(shown: boolean): void {
  write(STORAGE_KEYS.chatPreviews, shown);
}

/**
 * How many folded teams are kept. A team that is no longer listed leaves its
 * entry behind, so the oldest entries are dropped past this bound rather than
 * growing the key for as long as the app is installed.
 */
const FOLDED_TEAMS_MAX = 256;

/**
 * A team's key in the stored fold set. A store reference names the local
 * profile and aliases, so the key is a short hash of it rather than the
 * reference itself, and the preference writes no team identifier to disk. A
 * collision folds one more team, which a click undoes.
 */
function foldKey(team: string): string {
  return hashId(team);
}

function storedFoldKeys(): string[] {
  const raw = readStorage(STORAGE_KEYS.chatFoldedTeams);
  if (raw === null) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed)
      ? parsed
          .filter((key): key is string => typeof key === 'string')
          .slice(-FOLDED_TEAMS_MAX)
      : [];
  } catch {
    return [];
  }
}

/**
 * The stored fold set, read once: whether a team's channel list was folded
 * when the column last showed it.
 */
export function storedFoldedTeams(): (team: string) => boolean {
  const keys = new Set(storedFoldKeys());
  return (team) => keys.has(foldKey(team));
}

/** Records a team as folded or unfolded, most recent last. */
export function rememberTeamFolded(team: string, folded: boolean): void {
  const key = foldKey(team);
  const keys = storedFoldKeys().filter((entry) => entry !== key);
  if (folded) keys.push(key);
  writeStorage(
    STORAGE_KEYS.chatFoldedTeams,
    JSON.stringify(keys.slice(-FOLDED_TEAMS_MAX)),
  );
}
