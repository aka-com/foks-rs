/**
 * A group's mark: its initial drawn in the hue its store mark has (the
 * shell's `StoreHueContext`, falling back to a hue derived from its name),
 * over a tint of that hue (`.kico.tinted`), or the inactive gray. One mark for a group everywhere it is listed, so the Teams
 * row, the Files row, the Chat column and the group's own page agree.
 *
 * It lives in its own module because every list that draws a group draws it,
 * and none of them should have to pull in the group page to do so.
 */

import { useContext } from 'react';
import type { CSSProperties, ReactNode } from 'react';
import { StoreHueContext } from '../components/store-mark';
import { hue } from '../model';
import type { Store } from '../model';

export function GroupMark({
  store,
  size = 'md',
}: {
  store: Store;
  /** `sm` is the list row's 26px mark; `md` a sheet's; `big` the page hero's. */
  size?: 'sm' | 'md' | 'big';
}): ReactNode {
  const hues = useContext(StoreHueContext);
  return (
    <span
      className={['kico', size === 'sm' ? '' : size, 'group', 'tinted']
        .filter(Boolean)
        .join(' ')}
      // The initial stands for the name beside it; a row that read "E
      // Engineering" would say the name one and a half times.
      aria-hidden="true"
      style={
        {
          '--h':
            store.kind === 'team' && store.active === false
              ? 'var(--c-none)'
              : (hues?.get(store.id) ?? hue(store.name)),
        } as CSSProperties
      }
    >
      {store.name.slice(0, 1)}
    </span>
  );
}
