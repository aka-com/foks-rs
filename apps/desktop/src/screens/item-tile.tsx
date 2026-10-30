/**
 * Tinted kind tile for vault items: amber for passwords, accent for files, and
 * neutral for text documents. Shared across list rows, the details hero header,
 * and item sheets (`.tile` / `.bigtile`).
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

/** Resolves the tile class and icon for an existing item based on its kind. */
export function itemTile(item: Item): ItemTileFace {
  if (kindOf(item) === 'Password') return { className: 'pw', icon: 'key' };
  if (item.kind === 'File') return { className: 'file', icon: 'file' };
  return { className: 'text', icon: 'fileText' };
}

/** Resolves the tile class and icon for a new-item creation sheet. */
export function newItemTile(kind: 'Password' | 'Document'): ItemTileFace {
  return kind === 'Password'
    ? { className: 'pw', icon: 'key' }
    : { className: 'text', icon: 'fileText' };
}

/** Renders a 48px (`.hero`) or 36px (sheet header) item kind tile. */
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
