/**
 * Organising Files rows: the multi-selection the list keeps, the destination
 * checks the move sheet and a drag onto a folder share, and the sequential
 * runner behind the bulk progress sheet.
 *
 * Moves stay within one store: `MoveKv` relinks a dirent inside a store, and
 * moving between stores would mean re-encrypting under another team's keys.
 * Copy to vault does that instead, as a new item written by the host.
 */

import { normalizeCommandError } from '../../bridge';
import { displayPath, isTrashPath, nameOf } from '../../model';
import type { AgentSnapshot, Item } from '../../model';
import { failureOutcome } from '../../data/operation-outcome';

/* ---------------------------------------------------------------- paths -- */

/** The folder that holds `path`: `/` for a top-level entry. */
export function parentOf(path: string): string {
  const cut = path.lastIndexOf('/');
  return cut <= 0 ? '/' : path.slice(0, cut);
}

/** The last segment of `path` as stored, before any display decoding. */
export function baseName(path: string): string {
  return path.slice(path.lastIndexOf('/') + 1);
}

export function joinPath(folder: string, name: string): string {
  return folder === '/' ? `/${name}` : `${folder}/${name}`;
}

/** Whether `path` is `folder` itself or lies under it. */
export function within(path: string, folder: string): boolean {
  return folder === '/' || path === folder || path.startsWith(`${folder}/`);
}

/**
 * Whether `path` is an absolute folder path: `/`, or segments that are not
 * empty, `.` or `..`, with no trailing slash.
 */
export function validFolderPath(path: string): boolean {
  if (path === '/') return true;
  return (
    path.startsWith('/') &&
    path
      .split('/')
      .slice(1)
      .every((part) => part !== '' && part !== '.' && part !== '..')
  );
}

/** Whether `path` names an entry: a folder path other than the root. */
export function validItemPath(path: string): boolean {
  return path !== '/' && validFolderPath(path);
}

/** Why `name` cannot name an entry, or null. */
export function nameProblem(name: string): string | null {
  const trimmed = name.trim();
  if (!trimmed) return 'Enter a name.';
  if (trimmed.includes('/')) return 'A name cannot contain /.';
  if (trimmed === '.' || trimmed === '..')
    return 'Enter a name other than “.” or “..”.';
  return null;
}

/** "at the top level" or "in /env/prod", for messages about a folder. */
export function inFolder(folder: string): string {
  return folder === '/' ? 'at the top level' : `in ${displayPath(folder)}`;
}

/** "to the top level" or "to /env/prod". */
export function toFolder(folder: string): string {
  return folder === '/' ? 'to the top level' : `to ${displayPath(folder)}`;
}

export function itemCount(count: number): string {
  return `${count} ${count === 1 ? 'item' : 'items'}`;
}

function entryAt(
  snapshot: AgentSnapshot,
  store: string,
  path: string,
): Item | undefined {
  return snapshot.items.find(
    (item) => item.store === store && item.path === path,
  );
}

/**
 * Whether `path` is a folder of `store`: the root, a folder entry in the
 * catalog, a folder that holds catalog entries, or one of `known`, the
 * folders created since the catalog was read.
 */
export function folderExists(
  snapshot: AgentSnapshot,
  store: string,
  path: string,
  known: readonly string[] = [],
): boolean {
  if (path === '/' || known.includes(path)) return true;
  return snapshot.items.some(
    (item) =>
      item.store === store &&
      (item.kind === 'Folder'
        ? item.path === path
        : item.path.startsWith(`${path}/`)),
  );
}

/** The first entry on the way down to `folder` that is not a folder. */
function fileOnPath(
  snapshot: AgentSnapshot,
  store: string,
  folder: string,
): Item | undefined {
  if (folder === '/') return undefined;
  const parts = folder.split('/').slice(1);
  for (let depth = 1; depth <= parts.length; depth++) {
    const entry = entryAt(
      snapshot,
      store,
      `/${parts.slice(0, depth).join('/')}`,
    );
    if (entry && entry.kind !== 'Folder') return entry;
  }
  return undefined;
}

/* --------------------------------------------------------------- checks -- */

/** Why `item` cannot be copied to another vault at all, or undefined. */
export function copyProblem(item: Item): string | undefined {
  if (item.kind === 'Folder')
    return 'Folders cannot be copied to another vault. Copy the items in it one at a time.';
  if (isTrashPath(item.path))
    return 'Restore this item from Trash before copying it.';
  return undefined;
}

/**
 * What a move would do, or why it cannot be sent. `creates` names the folder
 * that does not exist yet and is created (with its parents) before the move.
 */
export type MoveCheck =
  | { ok: true; summary: string; creates: string | null }
  | { ok: false; problem: string };

const TRASH_DESTINATION =
  'Items go into Trash through Move to Trash, not through a move.';

function folderProblem(
  snapshot: AgentSnapshot,
  store: string,
  folder: string,
): string | null {
  if (isTrashPath(folder)) return TRASH_DESTINATION;
  const file = fileOnPath(snapshot, store, folder);
  return file ? `${displayPath(file.path)} is an item, not a folder.` : null;
}

/** Checks moving or renaming one item to the full path `destination`. */
export function checkMove(
  snapshot: AgentSnapshot,
  item: Item,
  destination: string,
  known: readonly string[] = [],
): MoveCheck {
  if (!validItemPath(destination))
    return {
      ok: false,
      problem:
        'Enter a path that starts with /, such as /logins/github.com, without empty, “.” or “..” parts.',
    };
  if (destination === item.path)
    return { ok: false, problem: 'Choose another folder or change the name.' };
  if (item.kind === 'Folder' && within(destination, item.path))
    return { ok: false, problem: 'A folder cannot move into itself.' };
  const folder = parentOf(destination);
  const problem = folderProblem(snapshot, item.store, folder);
  if (problem) return { ok: false, problem };
  // The name is Trash's own at the top level, whether or not Trash exists.
  if (isTrashPath(destination))
    return { ok: false, problem: TRASH_DESTINATION };
  if (entryAt(snapshot, item.store, destination))
    return {
      ok: false,
      problem: `Something named ${nameOf(destination)} is already ${inFolder(folder)}.`,
    };
  const creates = folderExists(snapshot, item.store, folder, known)
    ? null
    : folder;
  const name = nameOf(item.path);
  if (folder === parentOf(item.path))
    return {
      ok: true,
      summary: `Renames ${name} to ${nameOf(destination)}`,
      creates,
    };
  return {
    ok: true,
    summary: `${creates ? `Creates ${displayPath(creates)} and moves` : 'Moves'} ${name} to ${displayPath(destination)}`,
    creates,
  };
}

/** Checks moving several items of one store into `folder`, keeping names. */
export function checkMoveMany(
  snapshot: AgentSnapshot,
  items: readonly Item[],
  folder: string,
  known: readonly string[] = [],
): MoveCheck {
  const store = items[0]?.store;
  if (store === undefined)
    return { ok: false, problem: 'Select the items to move.' };
  if (items.some((item) => item.store !== store))
    return {
      ok: false,
      problem: 'Items move only within their own vault.',
    };
  if (!validFolderPath(folder))
    return {
      ok: false,
      problem:
        'Enter a folder path that starts with /, such as /logins, without empty, “.” or “..” parts.',
    };
  if (items.some((item) => item.kind === 'Folder' && within(folder, item.path)))
    return { ok: false, problem: 'A folder cannot move into itself.' };
  const problem = folderProblem(snapshot, store, folder);
  if (problem) return { ok: false, problem };
  const moving = items.filter((item) => parentOf(item.path) !== folder);
  if (!moving.length)
    return {
      ok: false,
      problem: `${items.length === 1 ? 'It is' : `All ${items.length} are`} already ${inFolder(folder)}.`,
    };
  const creates = folderExists(snapshot, store, folder, known) ? null : folder;
  const there = items.length - moving.length;
  return {
    ok: true,
    summary: `${creates ? `Creates ${displayPath(creates)} and moves` : 'Moves'} ${itemCount(moving.length)} ${toFolder(folder)}${there ? `; ${there} already ${there === 1 ? 'is' : 'are'} there` : ''}`,
    creates,
  };
}

/** What dropping dragged rows on a folder would do. */
export interface DropVerdict {
  ok: boolean;
  label: string;
}

/**
 * Judges a drop of `items` on `folder` of `store`. `storeName` names the
 * items' own vault; `blocked` is why that vault takes no writes, if it does
 * not.
 */
export function dropVerdict(
  items: readonly Item[],
  store: string,
  folder: string,
  storeName: string,
  blocked: string | null,
): DropVerdict {
  const what =
    items.length === 1 ? nameOf(items[0].path) : itemCount(items.length);
  if (items.some((item) => item.store !== store))
    return { ok: false, label: `Items move only within ${storeName}` };
  if (blocked) return { ok: false, label: blocked };
  if (items.some((item) => item.kind === 'Folder' && within(folder, item.path)))
    return { ok: false, label: 'A folder cannot move into itself' };
  if (items.every((item) => parentOf(item.path) === folder))
    return { ok: false, label: `Already ${inFolder(folder)}` };
  return {
    ok: true,
    label: `Move ${what} ${folder === '/' ? `to the top level of ${storeName}` : toFolder(folder)}`,
  };
}

/* ------------------------------------------------------- multi-selection -- */

/**
 * Rows of the Files list chosen together: keys in the order they were
 * chosen, and the row a Shift-click extends from.
 */
export interface MultiSelection {
  readonly keys: readonly string[];
  readonly anchor: string | null;
}

export const NO_ROWS: MultiSelection = { keys: [], anchor: null };

export function rowKey(item: { store: string; path: string }): string {
  return `${item.store}|${item.path}`;
}

/**
 * ⌘-click: adds or removes one row. `seed` is the row the details panel
 * shows; it joins the first time rows are chosen this way, as it was
 * selected already.
 */
export function toggleRow(
  selection: MultiSelection,
  key: string,
  seed?: string | null,
): MultiSelection {
  const base =
    selection.keys.length === 0 && seed && seed !== key
      ? [seed]
      : selection.keys;
  return {
    keys: base.includes(key)
      ? base.filter((entry) => entry !== key)
      : [...base, key],
    anchor: key,
  };
}

/**
 * Shift-click: every listed row from the anchor (or `seed`, the row the
 * details panel shows) through `key`.
 */
export function extendRows(
  selection: MultiSelection,
  listed: readonly string[],
  key: string,
  seed?: string | null,
): MultiSelection {
  const to = listed.indexOf(key);
  if (to < 0) return selection;
  const anchor = selection.anchor ?? seed ?? key;
  const from = listed.indexOf(anchor);
  if (from < 0) return { keys: [key], anchor: key };
  return {
    keys: listed.slice(Math.min(from, to), Math.max(from, to) + 1),
    anchor,
  };
}

/** ⌘A: every listed row. */
export function allRows(listed: readonly string[]): MultiSelection {
  return { keys: [...listed], anchor: listed[0] ?? null };
}

/** The chosen rows that are still listed, in list order. */
export function selectedRows<T extends { store: string; path: string }>(
  selection: MultiSelection,
  listed: readonly T[],
): T[] {
  const keys = new Set(selection.keys);
  return listed.filter((item) => keys.has(rowKey(item)));
}

/* ---------------------------------------------------------------- runner -- */

/**
 * One item's state in a bulk run. `failed` items were not changed and can be
 * retried; `refused` items were declined for a reason a retry does not
 * change; `uncertain` items may or may not have changed.
 */
export type BulkStatus =
  | 'pending'
  | 'running'
  | 'done'
  | 'skipped'
  | 'refused'
  | 'failed'
  | 'uncertain';

export interface BulkRow {
  readonly item: Item;
  readonly status: BulkStatus;
  readonly message: string;
}

export function pendingRows(items: readonly Item[]): BulkRow[] {
  return items.map((item) => ({ item, status: 'pending', message: 'Waiting' }));
}

/** One bulk operation: how it reads while running, and what it does. */
export interface BulkAction {
  running: string;
  /** Why the current version of the item is not sent, or null to send it. */
  check: (
    item: Item,
    snapshot: AgentSnapshot,
  ) => { status: 'skipped' | 'refused'; message: string } | null;
  /** Sends one request and resolves with the row's message. */
  apply: (item: Item) => Promise<string>;
  /**
   * The current catalog entry a row acts on. Without it, the entry at the
   * row's path with the row's dirent id.
   */
  locate?: (item: Item, snapshot: AgentSnapshot) => Item | undefined;
}

/** Moving rows into `folder` of their store, keeping their names. */
export function moveRowCheck(
  item: Item,
  snapshot: AgentSnapshot,
  folder: string,
): { status: 'skipped' | 'refused'; message: string } | null {
  if (parentOf(item.path) === folder)
    return { status: 'skipped', message: `Already ${inFolder(folder)}` };
  if (item.kind === 'Folder' && within(folder, item.path))
    return { status: 'refused', message: 'A folder cannot move into itself.' };
  const destination = joinPath(folder, baseName(item.path));
  if (entryAt(snapshot, item.store, destination))
    return {
      status: 'refused',
      message: `Something named ${nameOf(destination)} is already ${inFolder(folder)}.`,
    };
  return null;
}

/**
 * Runs `action` on the rows at `indexes`, one at a time. Every write retires
 * the host's catalog of its store, and the host selects each item in a
 * catalog it has read, so the catalog is read again before each item and the
 * item's current version is sent. An item whose entry was replaced or
 * removed meanwhile is not sent. A failure is classified by whether the
 * request may have run: only one that certainly did not is offered again.
 * The run stops when the catalog cannot be read, marking what is left as
 * not started.
 */
export async function runBulk({
  rows,
  indexes,
  action,
  reload,
  onRows,
  onFailure,
}: {
  rows: readonly BulkRow[];
  indexes: readonly number[];
  action: BulkAction;
  reload: () => Promise<AgentSnapshot>;
  onRows: (rows: readonly BulkRow[]) => void;
  onFailure: (error: unknown, item: Item) => Promise<void>;
}): Promise<readonly BulkRow[]> {
  let current: readonly BulkRow[] = rows;
  const set = (index: number, status: BulkStatus, message: string): void => {
    current = current.map((row, at) =>
      at === index ? { ...row, status, message } : row,
    );
    onRows(current);
  };
  for (const [position, index] of indexes.entries()) {
    const row = current[index];
    if (!row) continue;
    set(index, 'running', action.running);
    let snapshot: AgentSnapshot;
    try {
      snapshot = await reload();
    } catch (error) {
      const { message } = normalizeCommandError(error);
      for (const rest of indexes.slice(position))
        set(rest, 'failed', `Not started: ${message}`);
      return current;
    }
    const fresh = action.locate
      ? action.locate(row.item, snapshot)
      : snapshot.items.find(
          (item) =>
            item.store === row.item.store &&
            item.path === row.item.path &&
            item.direntId === row.item.direntId,
        );
    if (!fresh) {
      set(
        index,
        'refused',
        'This item was changed or removed elsewhere. Refresh to review it.',
      );
      continue;
    }
    const verdict = action.check(fresh, snapshot);
    if (verdict) {
      set(index, verdict.status, verdict.message);
      continue;
    }
    try {
      set(index, 'done', await action.apply(fresh));
    } catch (error) {
      const typed = normalizeCommandError(error);
      const outcome = failureOutcome(typed);
      if (outcome === 'unknown')
        set(
          index,
          'uncertain',
          `${typed.message} It may have changed; refresh to check before trying again.`,
        );
      else if (outcome === 'not-started' || typed.retryable)
        set(index, 'failed', typed.message);
      else set(index, 'refused', typed.message);
      await onFailure(error, fresh);
    }
  }
  return current;
}

/** Counts of rows by status. */
export function bulkCounts(
  rows: readonly BulkRow[],
): Readonly<Record<BulkStatus, number>> {
  const counts: Record<BulkStatus, number> = {
    pending: 0,
    running: 0,
    done: 0,
    skipped: 0,
    refused: 0,
    failed: 0,
    uncertain: 0,
  };
  for (const row of rows) counts[row.status]++;
  return counts;
}
