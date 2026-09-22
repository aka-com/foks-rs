/**
 * Client-side item classification and display metadata.
 *
 * Classifies filesystem nodes into two user-facing product kinds:
 *   - Password: Secret node containing a password field or located under `/logins/`
 *   - Document: any other Secret node, and every File node
 *
 * The v0.1.9 KV format stores every value up to 2,040 bytes as one `small-file`
 * node whether it was typed into the app or brought in from disk, so a note
 * and a small file are the same thing to the protocol and are one kind here.
 * Whether an item shows a value or a download is decided by its node kind, not
 * its product kind. Symlink nodes are dropped when the agent's snapshot is
 * decoded and never reach this module.
 */

import type { Item, ItemKind } from './types';
import type { FoksIconName } from '../icon-name';

export interface KindMeta {
  /** Display label for the kind. */
  label: string;
  plural: string;
  /** Type-only dependency keeps invalid display metadata out of the UI. */
  icon: FoksIconName;
  blurb: string;
}

export const KINDS: Readonly<Record<'Password' | 'Document', KindMeta>> = {
  Password: {
    label: 'Password',
    plural: 'Passwords',
    icon: 'key',
    blurb: 'Keep passwords, secrets, and tokens here.',
  },
  Document: {
    label: 'Document',
    plural: 'Documents',
    icon: 'file',
    blurb: 'Keep notes, keys, and files in encrypted storage.',
  },
};

/** The word a person sees for a kind. */
export function kindLabel(kind: keyof typeof KINDS): string {
  return KINDS[kind].label;
}

/** The filter order the kind segmented control uses. */
export const KIND_LIST = Object.keys(KINDS) as (keyof typeof KINDS)[];

const PASSWORD_LINE = /^password:/m;

/** The kind a person reads this item as. */
export function kindOf(item: Pick<Item, 'kind' | 'path' | 'value'>): ItemKind {
  if (item.kind === 'Folder') return 'Folder';
  if (
    item.kind === 'Secret' &&
    (PASSWORD_LINE.test(item.value ?? '') || item.path.startsWith('/logins/'))
  )
    return 'Password';
  return 'Document';
}

/** A Password that is also filed under `/logins/` — the site-tile case. */
export function isLogin(item: Pick<Item, 'kind' | 'path' | 'value'>): boolean {
  return kindOf(item) === 'Password' && item.path.startsWith('/logins/');
}

/** Decodes one canonical KV path component for display without changing its identity. */
export function displayPathComponent(component: string): string {
  if (!component.includes('%')) return component;
  try {
    return [...decodeURIComponent(component)]
      .map((character) => {
        const code = character.codePointAt(0) ?? 0;
        return code < 32 || code === 127 ? '�' : character;
      })
      .join('');
  } catch {
    // Keep malformed or non-UTF-8 components visible and reversible instead
    // of guessing at bytes that the UI cannot faithfully represent.
    return component;
  }
}

/** Decodes every component of a canonical KV path for display. */
export function displayPath(path: string): string {
  if (path === '/') return path;
  return path.split('/').map(displayPathComponent).join('/');
}

/** The decoded last segment of a path, or `/` at the root. */
export function nameOf(path: string): string {
  const name = path.slice(path.lastIndexOf('/') + 1);
  return name ? displayPathComponent(name) : '/';
}

/** The decoded folder path without its leading slash and last segment. */
export function prefixOf(path: string): string {
  // Return an empty prefix for paths without directory separators.
  const cut = path.lastIndexOf('/');
  return cut <= 0 ? '' : displayPath(path.slice(1, cut));
}
