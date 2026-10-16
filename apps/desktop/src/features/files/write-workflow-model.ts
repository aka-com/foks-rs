/**
 * The write workflow the shell's overlay shows: which new-item, move, copy,
 * import, export, Trash or conflict sheet is open and what it resumes from.
 * These are the pure parts of `write-workflows.tsx`: the workflow and draft
 * types, the paths a dropped file takes, the sheet an edit conflict resumes
 * in, and the workflow a `?state=` scene or a command error opens.
 */

import { normalizeCommandError } from '../../bridge';
import type { KvRoleInput } from '../../bridge';
import {
  isLogin,
  itemKey,
  kindOf,
  loginField,
  loginFieldOf,
  loginLines,
  nameOf,
} from '../../model';
import type {
  AgentSnapshot,
  Item,
  ItemKind,
  LoginFieldName,
  StoreRef,
} from '../../model';
import { DEFAULT_READ_ROLE, DEFAULT_WRITE_ROLE } from './item-roles';

export type NewKind = Exclude<ItemKind, 'Folder'>;

/** Default item kind used when opening the new-item sheet without one. */
export const DEFAULT_NEW_KIND: NewKind = 'Password';

/** A partially filled new-item form, carried across conflict recovery. */
export interface NewDraft {
  path: string;
  site: string;
  username: string;
  password: string;
  website: string;
  value: string;
  resourceName: string;
  sourcePath: string | null;
  readRole: KvRoleInput;
  writeRole: KvRoleInput;
}

export type WriteWorkflow =
  | {
      kind: 'new';
      itemKind: NewKind;
      storeId: StoreRef;
      initialFolder?: string;
      draft?: NewDraft;
    }
  | {
      kind: 'new-folder';
      storeId: StoreRef;
      initialFolder?: string;
    }
  | {
      kind: 'exists';
      itemKind: NewKind;
      storeId: StoreRef;
      path: string;
      initialFolder?: string;
      draft?: NewDraft;
    }
  | { kind: 'move'; item: Item }
  /** Copies an item into another vault, optionally moving the original to Trash. */
  | { kind: 'copy'; item: Item }
  /**
   * Imports logins from another password manager's CSV export, into
   * `storeId` unless another vault is chosen.
   */
  | { kind: 'import'; storeId: StoreRef; initialFolder?: string }
  /** Writes the passwords of `storeId` to a plain-text CSV file. */
  | { kind: 'export'; storeId: StoreRef }
  /** Several items of one store, moved together to a folder chosen next. */
  | { kind: 'move-many'; items: readonly Item[] }
  /** Moves `items` into `folder` of their store one at a time, with progress. */
  | { kind: 'bulk-move'; items: readonly Item[]; folder: string }
  /** Moves `items` to their stores' Trash one at a time, with progress. */
  | { kind: 'bulk-trash'; items: readonly Item[] }
  /**
   * Moves Trash entries back to where `items` were, one at a time, with
   * progress: the Undo of a bulk-trash run. `trash[i]` is the path in Trash
   * of `items[i]`.
   */
  | {
      kind: 'bulk-restore';
      items: readonly Item[];
      trash: readonly string[];
    }
  /**
   * Permanently removes an entry in Trash, or with `outsideTrash` an item in
   * a team whose Trash this account cannot set up.
   */
  | { kind: 'delete'; item: Item; outsideTrash?: boolean }
  /** Restoring a Trash entry found another item at `destination`. */
  | { kind: 'restore-conflict'; item: Item; destination: string }
  | { kind: 'empty-trash'; store: StoreRef }
  | {
      kind: 'conflict';
      item: Item;
      draft: string;
      operation?: 'edit' | 'replace';
    }
  | null;

/** What the Path field shows while it is empty. */
export const PATH_HINT: Readonly<Record<NewKind, string>> = {
  Password: '/logins/github.com',
  Document: '/passport-scan.pdf',
};

/** The vault path a dropped file takes: the current folder, else the root. */
export function fileDropPath(name: string, folder?: string): string {
  return namedPath(name, '/', folder);
}

/**
 * `name` inside `folder`, or inside `defaultDirectory` when no folder is open
 * or the open folder is the root.
 */
export function namedPath(
  name: string,
  defaultDirectory: string,
  folder?: string,
): string {
  const directory = (
    folder && folder !== '/' ? folder : defaultDirectory
  ).replace(/\/$/, '');
  return `${directory}/${name}`;
}

/**
 * The new-item draft a dropped file resumes from, so a drop that lands on an
 * occupied path can reopen the sheet with the file and path already chosen.
 */
export function droppedFileDraft(path: string, sourcePath: string): NewDraft {
  return {
    path,
    site: '',
    username: '',
    password: '',
    website: '',
    value: '',
    resourceName: '',
    sourcePath,
    readRole: DEFAULT_READ_ROLE,
    writeRole: DEFAULT_WRITE_ROLE,
  };
}

/**
 * Whether the New password sheet holds all of this text: every line that is
 * not empty names a different one of its three fields.
 */
function fitsPasswordSheet(text: string): boolean {
  const named = new Set<LoginFieldName>();
  return loginLines(text).every(({ key, raw }) => {
    if (key === null && (raw === '' || raw === '\r')) return true;
    const field = loginFieldOf(key);
    if (field === null || named.has(field)) return false;
    named.add(field);
    return true;
  });
}

/**
 * The new-item sheet an edit resumes in when the item it was editing has been
 * deleted elsewhere: there is no newer version to review, so the edit is
 * offered as a new item at the same path, for the user to save or discard.
 * A login resumes in the Password sheet only when that sheet's fields hold
 * the whole draft; otherwise it resumes as a Document with the draft as its
 * value, so no line is dropped.
 */
export function conflictDraftWorkflow(
  item: Item,
  draft: string,
): WriteWorkflow {
  const itemKind: NewKind =
    kindOf(item) === 'Password' && fitsPasswordSheet(draft)
      ? 'Password'
      : 'Document';
  const field = (name: LoginFieldName): string =>
    itemKind === 'Password' ? (loginField(draft, name) ?? '') : '';
  return {
    kind: 'new',
    itemKind,
    storeId: item.store,
    draft: {
      path: item.path,
      site: itemKind === 'Password' ? nameOf(item.path) : '',
      username: field('username'),
      password: field('password'),
      website: field('url'),
      value: itemKind === 'Password' ? '' : draft,
      resourceName: '',
      sourcePath: null,
      readRole: DEFAULT_READ_ROLE,
      writeRole: DEFAULT_WRITE_ROLE,
    },
  };
}

export function initialWriteWorkflow(
  search: string,
  snapshot?: AgentSnapshot,
): WriteWorkflow {
  const state = new URLSearchParams(search).get('state');
  if (state === 'new')
    return { kind: 'new', itemKind: 'Password', storeId: 'team:household' };
  if (state === 'new-group')
    return { kind: 'new', itemKind: 'Document', storeId: 'team:eng' };
  if (state === 'new-document')
    return { kind: 'new', itemKind: 'Document', storeId: 'acct:personal' };
  if (state === 'group-new-document')
    return { kind: 'new', itemKind: 'Document', storeId: 'team:eng' };
  if (state === 'exists') {
    return {
      kind: 'exists',
      itemKind: 'Password',
      storeId: 'acct:personal',
      path: PATH_HINT.Password,
      initialFolder: '/logins',
    };
  }
  if (state === 'conflict' && snapshot) {
    const item = snapshot.items.find((candidate) => isLogin(candidate));
    if (item)
      return {
        kind: 'conflict',
        item,
        draft: snapshot.plaintext[itemKey(item)] ?? '',
      };
  }
  return null;
}

export function workflowForError(
  error: unknown,
  item?: Item,
  draft = '',
): WriteWorkflow | null {
  const typed = normalizeCommandError(error);
  if (typed.code === 'conflict' && item)
    return { kind: 'conflict', item, draft };
  return null;
}
