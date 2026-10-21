/**
 * The tinted kind tile an item is drawn with: amber for a password, the
 * accent for a file, neutral for a typed document. List rows, the details
 * hero and the item sheets use the same one (`.tile` / `.bigtile`).
 */

import type { ReactNode } from 'react';
import { Icon } from '../components';
import type { FoksIconName } from '../icons';
import { kindOf } from '../model';
import type { Item } from '../model';

export interface ItemTileFace {
  className: 'pw' | 'file' | 'text';
  icon: FoksIconName;
}

/** The tile class and glyph for an item, by its kind and node kind. */
export function itemTile(item: Item): ItemTileFace {
  if (kindOf(item) === 'Password') return { className: 'pw', icon: 'key' };
  if (item.kind === 'File') return { className: 'file', icon: 'file' };
  return { className: 'text', icon: 'fileText' };
}

/** The tile for a kind about to be created, before there is an item. */
export function newItemTile(kind: 'Password' | 'Document'): ItemTileFace {
  return kind === 'Password'
    ? { className: 'pw', icon: 'key' }
    : { className: 'text', icon: 'fileText' };
}

/** A 48px (`hero`) or 36px (sheet header) tile. */
export function BigTile({
  face,
  className,
}: {
  face: ItemTileFace;
  className?: string;
}): ReactNode {
  return (
    <span
      className={['bigtile', face.className, className ?? '']
        .filter(Boolean)
        .join(' ')}
      aria-hidden="true"
    >
      <Icon name={face.icon} />
    </span>
  );
}
