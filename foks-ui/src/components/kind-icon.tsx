/**
 * The kind glyphs — `shell.js`'s `kic()` and `kico()`.
 *
 * `KindIcon` is the tinted square a row, a details header and a menu item
 * carry: the kind's icon in the kind's colour on a wash of it. `KindGlyph` is
 * the solid tile-sized square, which for a login under `/logins/` becomes the
 * site's initial on its own hue instead — the design's one favicon-shaped
 * affordance.
 *
 * The kind itself is never passed in as a string by a caller: it comes from
 * `kindOf`, the client-side reading of the node type.
 */

import type { ReactNode } from 'react';
import { Icon } from './icon';
import { KINDS, hue, isLogin, kindOf, nameOf } from '../model';
import type { Item } from '../model';
import type { FoksIconName } from '../icons';

/** The four kinds a person filters and files by. Folders are not items. */
export type FilterKind = keyof typeof KINDS;

export interface KindIconProps {
  kind: FilterKind;
  /** `md` in a details header, `big` in an empty state. */
  className?: string;
}

export function KindIcon({ kind, className }: KindIconProps): ReactNode {
  return (
    <span className={['kic', kind, className].filter(Boolean).join(' ')}>
      <Icon name={KINDS[kind].icon as FoksIconName} />
    </span>
  );
}

export interface KindGlyphProps {
  item: Item;
  size?: 'md' | 'big';
}

export function KindGlyph({ item, size }: KindGlyphProps): ReactNode {
  const classes = ['kico', size].filter(Boolean);
  if (isLogin(item)) {
    const site = nameOf(item.path);
    return (
      <span
        className={classes.join(' ')}
        style={{ background: hue(site) }}
        aria-hidden="true"
      >
        {site[0].toUpperCase()}
      </span>
    );
  }
  const kind = kindOf(item) as FilterKind;
  return (
    <span className={[...classes, kind].join(' ')}>
      <Icon name={KINDS[kind].icon as FoksIconName} />
    </span>
  );
}
