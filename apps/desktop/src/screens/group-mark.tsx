/**
 * Renders a group mark: the group's initial in its store mark hue (from
 * `StoreHueContext`, falling back to a name-derived hue) on a tinted
 * background (`.kico.tinted`), or inactive gray. Shared by the Teams list,
 * Files list, Chat column, and group page so each group renders a consistent
 * mark without importing the group page module.
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
