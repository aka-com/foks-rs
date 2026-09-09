/**
 * Client-side item classification and display metadata.
 *
 * Classifies filesystem nodes into user-facing product kinds:
 *   - Password: Secret node containing a password field or located under `/logins/`
 *   - Resource: Any other Secret node
 *   - File: File node
 *   - Link: Symlink node
 */

import type { Item, ItemKind, NodeKind, NodeType } from './types';

export interface KindMeta {
  /** Display label for the kind (e.g. Note for Resource). */
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
    blurb: 'Keep links and shortcuts to other items in this vault.',
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

/** Maps an item kind to its underlying filesystem node type. */
export function rtype(item: Pick<Item, 'kind'>): NodeType {
  return NODE_TYPES[item.kind];
}

const NODE_WORDS: Readonly<Record<NodeKind, string>> = {
  Secret: 'a Secret',
  File: 'a file',
  Link: 'a link',
  Folder: 'a folder',
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
  // Return an empty prefix for paths without directory separators.
  const cut = path.lastIndexOf('/');
  return cut <= 0 ? '' : path.slice(1, cut);
}
