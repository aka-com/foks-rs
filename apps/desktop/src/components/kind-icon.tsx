/**
 * Item kind icon and glyph components.
 *
 * `KindIcon` displays an icon tinted with the color designated for that kind.
 * `KindGlyph` displays a tile glyph, rendering the website's initial on a deterministic
 * colored background for login credentials under `/logins/`.
 */

import type { ReactNode } from 'react';
import { Icon } from './icon';
import { KINDS, hue, isLogin, kindOf, nameOf } from '../model';
import type { Item } from '../model';

/** Item kinds available for filtering and creation. */
export type FilterKind = keyof typeof KINDS;

export interface KindIconProps {
  kind: FilterKind;
  /** `md` in a details header, `big` in an empty state. */
  className?: string;
}

export function KindIcon({ kind, className }: KindIconProps): ReactNode {
  return (
    <span className={['kic', kind, className].filter(Boolean).join(' ')}>
      <Icon name={KINDS[kind].icon} />
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
      <Icon name={KINDS[kind].icon} />
    </span>
  );
}
