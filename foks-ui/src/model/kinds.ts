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
  /** The singular a person reads — Resource is shown as Note. */
  label: string;
  plural: string;
  /** A `FoksIconName`; kept as a plain string so the model imports no view. */
  icon: string;
  blurb: string;
}

export const KINDS: Readonly<
  Record<'Password' | 'Resource' | 'File' | 'Link', KindMeta>
> = {
  Password: {
    label: 'Password',
    plural: 'Passwords',
    icon: 'key',
    blurb: 'Keep passwords, secrets, and tokens here.',
  },
  Resource: {
    label: 'Note',
    plural: 'Notes',
    icon: 'term',
    blurb: 'Keep keys, tokens, connection strings, and notes here.',
  },
  File: {
    label: 'File',
    plural: 'Files',
    icon: 'file',
    blurb: 'Keep documents and files in encrypted storage.',
  },
  Link: {
    label: 'Link',
    plural: 'Links',
    icon: 'link',
    blurb: 'Keep links and shortcuts to other paths in the store.',
  },
};

/** The word a person sees for a kind. Resource is Note. */
export function kindLabel(kind: keyof typeof KINDS): string {
  return KINDS[kind].label;
}

/** The filter order the kind segmented control uses. */
export const KIND_LIST = Object.keys(KINDS) as (keyof typeof KINDS)[];

const PASSWORD_LINE = /^password:/m;

/** The kind a person reads this item as. */
export function kindOf(item: Pick<Item, 'kind' | 'path' | 'value'>): ItemKind {
  if (item.kind !== 'Secret') return item.kind;
  return PASSWORD_LINE.test(item.value ?? '') ||
    item.path.startsWith('/logins/')
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
  // A path with no `/` has no prefix. `lastIndexOf` gives -1 for it, and
  // `slice(1, -1)` then chopped the first and last character off the name.
  const cut = path.lastIndexOf('/');
  return cut <= 0 ? '' : path.slice(1, cut);
}
