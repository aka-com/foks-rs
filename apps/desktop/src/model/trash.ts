/**
 * Vault Trash, as the catalog shows it.
 *
 * Deleting an item moves it into `/.trash/<role-tag>/` in the same store, one
 * folder per read role, under a name that records when it was deleted and
 * where it was: `<UTC yyyymmddThhmmss>~<percent-encoded original path>`. A
 * name that would pass the 255-byte limit is `<stamp>~#<hash>` instead, with
 * the original path in a small file named `<that name>.origin` beside it. The
 * host writes these names; this module reads them back from catalog paths.
 * Nothing under `/.trash` is part of the ordinary folders, All items or
 * search.
 */

import { parseRole } from './roles';
import type { AgentSnapshot, Item, StoreRef } from './types';

/** The folder that holds every role's Trash folder in a store. */
export const TRASH_ROOT = '/.trash';

/** The suffix of the small file that records a hashed entry's original path. */
export const ORIGIN_SUFFIX = '.origin';

/** Whether `path` is the Trash root or lies under it. */
export function isTrashPath(path: string): boolean {
  return path === TRASH_ROOT || path.startsWith(`${TRASH_ROOT}/`);
}

/** Encodes `%`, `/` and control characters, as the host does. */
export function encodeOriginalPath(path: string): string {
  let encoded = '';
  for (const character of path) {
    const code = character.codePointAt(0) ?? 0;
    encoded +=
      character === '%' || character === '/' || code < 0x20 || code === 0x7f
        ? `%${code.toString(16).toUpperCase().padStart(2, '0')}`
        : character;
  }
  return encoded;
}

function decodeOriginalPath(encoded: string): string | null {
  const bytes: number[] = [];
  const raw = new TextEncoder().encode(encoded);
  for (let index = 0; index < raw.length; index += 1) {
    if (raw[index] !== 0x25) {
      bytes.push(raw[index]);
      continue;
    }
    const hex = String.fromCharCode(...raw.subarray(index + 1, index + 3));
    if (!/^[0-9a-fA-F]{2}$/.test(hex)) return null;
    bytes.push(parseInt(hex, 16));
    index += 2;
  }
  try {
    return new TextDecoder('utf-8', { fatal: true }).decode(
      new Uint8Array(bytes),
    );
  } catch {
    return null;
  }
}

/** `yyyymmddThhmmss` in UTC. */
export function trashStamp(at: Date): string {
  const two = (value: number): string => String(value).padStart(2, '0');
  return `${String(at.getUTCFullYear()).padStart(4, '0')}${two(at.getUTCMonth() + 1)}${two(at.getUTCDate())}T${two(at.getUTCHours())}${two(at.getUTCMinutes())}${two(at.getUTCSeconds())}`;
}

/** What a name in a role's Trash folder records. */
export type TrashName =
  | { kind: 'path'; deletedAt: number; original: string }
  | { kind: 'hashed'; deletedAt: number }
  | { kind: 'origin' };

const STAMP = /^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})$/;
const HASH = /^[0-9a-fA-F]{32}$/;

/** Reads a Trash entry name; `null` for a name the host did not write. */
export function parseTrashName(name: string): TrashName | null {
  const cut = name.indexOf('~');
  if (cut < 0) return null;
  const stamp = STAMP.exec(name.slice(0, cut));
  if (!stamp) return null;
  const [, year, month, day, hours, minutes, seconds] = stamp.map(Number);
  const deletedAt = Date.UTC(year, month - 1, day, hours, minutes, seconds);
  const rest = name.slice(cut + 1);
  if (rest.startsWith('#')) {
    const hash = rest.slice(1);
    if (HASH.test(hash)) return { kind: 'hashed', deletedAt };
    return hash.endsWith(ORIGIN_SUFFIX) &&
      HASH.test(hash.slice(0, -ORIGIN_SUFFIX.length))
      ? { kind: 'origin' }
      : null;
  }
  const original = decodeOriginalPath(rest);
  return original?.startsWith('/')
    ? { kind: 'path', deletedAt, original }
    : null;
}

/** One entry in a store's Trash: an item or a folder moved there whole. */
export interface TrashEntry {
  item: Item;
  /** The entry's name in Trash, as stored. */
  name: string;
  /** The role folder the entry is in, such as `/.trash/owner`. */
  folder: string;
  /** Where the entry was; `null` while a hashed entry's `.origin` is unread. */
  original: string | null;
  /** Milliseconds since the epoch; `null` for a name the host did not write. */
  deletedAt: number | null;
  /** The `.origin` file of a hashed entry, when the catalog lists it. */
  origin?: Item;
  /** For a folder, how many items it holds. */
  count: number;
}

/**
 * The entries directly in each role folder of `store`'s Trash, newest first.
 * `.origin` files are listed with their entries rather than on their own.
 */
export function trashEntries(
  snapshot: AgentSnapshot,
  store: StoreRef,
): TrashEntry[] {
  const inStore = snapshot.items.filter(
    (item) => item.store === store && item.path.startsWith(`${TRASH_ROOT}/`),
  );
  const byPath = new Map(inStore.map((item) => [item.path, item]));
  const entries: TrashEntry[] = [];
  for (const item of inStore) {
    const parts = item.path.slice(TRASH_ROOT.length + 1).split('/');
    if (parts.length !== 2 || !parts[0] || !parts[1]) continue;
    const name = parts[1];
    const parsed = parseTrashName(name);
    if (parsed?.kind === 'origin') continue;
    const origin =
      parsed?.kind === 'hashed'
        ? byPath.get(`${item.path}${ORIGIN_SUFFIX}`)
        : undefined;
    entries.push({
      item,
      name,
      folder: `${TRASH_ROOT}/${parts[0]}`,
      original: parsed?.kind === 'path' ? parsed.original : null,
      deletedAt: parsed ? parsed.deletedAt : null,
      ...(origin ? { origin } : {}),
      count:
        item.kind === 'Folder'
          ? inStore.filter(
              (child) =>
                child.kind !== 'Folder' &&
                child.path.startsWith(`${item.path}/`),
            ).length
          : 0,
    });
  }
  return entries.sort(
    (a, b) =>
      (b.deletedAt ?? 0) - (a.deletedAt ?? 0) || a.name.localeCompare(b.name),
  );
}

/**
 * Whether an entry in Trash is readable only by owners: it sits in the
 * owners' role folder, so only owners can list or restore it.
 */
export function ownersOnly(entry: TrashEntry): boolean {
  return parseRole(entry.item.read)?.kind === 'owner';
}

/** The last component of a path, which is what an entry is called. */
function lastComponent(path: string): string {
  return path.slice(path.lastIndexOf('/') + 1);
}

/** What an entry is called: its original name, or its name in Trash. */
export function trashEntryLabel(entry: TrashEntry): string {
  return entry.original ? lastComponent(entry.original) : entry.name;
}

/**
 * The first free path for restoring `original` as a copy beside the item
 * that now holds it: `<name> (restored)`, then `<name> (restored 2)`, and so
 * on.
 */
export function restoredCopyPath(
  original: string,
  taken: (path: string) => boolean,
): string {
  for (let attempt = 1; ; attempt += 1) {
    const candidate = `${original}${attempt === 1 ? ' (restored)' : ` (restored ${attempt})`}`;
    if (!taken(candidate)) return candidate;
  }
}
