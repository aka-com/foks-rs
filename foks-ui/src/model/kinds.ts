/**
 * The kind rule — ported from `wave6/shell.js:179-193`.
 *
 * A **client-side reading** of the node type with no protocol meaning
 * which is why it lives here and not in Rust: duplicating it
 * across the seam would create two truths.
 *
 *   Password = a Secret with a `password:` line, or under `/logins/`
 *   Resource = any other Secret
 *   File     = a File node
 *   Link     = a symlink
 *
 * Folders are not items: the path prefix is a chip on the row.
 */

import type { Item, ItemKind, NodeKind, NodeType } from './types';

export interface KindMeta {
  plural: string;
  /** A `FoksIconName`; kept as a plain string so the model imports no view. */
  icon: string;
  blurb: string;
}

export const KINDS: Readonly<Record<'Password' | 'Resource' | 'File' | 'Link', KindMeta>> = {
  Password: {
    plural: 'Passwords',
    icon: 'key',
    blurb: 'A login: user, password and website — read inline.',
  },
  Resource: {
    plural: 'Resources',
    icon: 'term',
    blurb: 'A token, key, connection string or note — read inline.',
  },
  File: {
    plural: 'Files',
    icon: 'file',
    blurb: 'A document or bundle — read in version-bound chunks.',
  },
  Link: {
    plural: 'Links',
    icon: 'link',
    blurb: 'A pointer to another path in the same store.',
  },
};

/** The filter order the kind segmented control uses. */
export const KIND_LIST = Object.keys(KINDS) as (keyof typeof KINDS)[];

const PASSWORD_LINE = /^password:/m;

/** The kind a person reads this item as. */
export function kindOf(item: Pick<Item, 'kind' | 'path' | 'value'>): ItemKind {
  if (item.kind !== 'Secret') return item.kind;
  return PASSWORD_LINE.test(item.value ?? '') || item.path.startsWith('/logins/')
    ? 'Password'
    : 'Resource';
}

const NODE_TYPES: Readonly<Record<NodeKind, NodeType>> = {
  Secret: 'small_file',
  File: 'file',
  Link: 'symlink',
  Folder: 'directory',
};

/** The node type the kind is a reading of. */
export function rtype(item: Pick<Item, 'kind'>): NodeType {
  return NODE_TYPES[item.kind];
}

const NODE_WORDS: Readonly<Record<NodeKind, string>> = {
  Secret: 'a Secret',
  File: 'a File node',
  Link: 'a symlink',
  Folder: 'a directory',
};

/** How the details panel names the node type in prose. */
export function rtypeWords(item: Pick<Item, 'kind'>): string {
  return NODE_WORDS[item.kind];
}

/** A Password that is also filed under `/logins/` — the site-tile case. */
export function isLogin(item: Pick<Item, 'kind' | 'path' | 'value'>): boolean {
  return kindOf(item) === 'Password' && item.path.startsWith('/logins/');
}

/** The last segment of a path, or `/` at the root. */
export function nameOf(path: string): string {
  return path.slice(path.lastIndexOf('/') + 1) || '/';
}

/** The folder chip: the path without its leading slash and last segment. */
export function prefixOf(path: string): string {
  return path.slice(1, path.lastIndexOf('/'));
}
