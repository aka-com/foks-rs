/**
 * A group's mark: its initial over a colour derived from its name, or the
 * inactive grey. One mark for a group everywhere it is listed, so the Teams
 * row, the Files row, the Chat column and the group's own page agree.
 *
 * It lives in its own module because every list that draws a group draws it,
 * and none of them should have to pull in the group page to do so.
 */

import type { ReactNode } from 'react';
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
  return (
    <span
      className={['kico', size === 'sm' ? '' : size, 'group']
        .filter(Boolean)
        .join(' ')}
      // The initial stands for the name beside it; a row that read "E
      // Engineering" would say the name one and a half times.
      aria-hidden="true"
      style={{
        background:
          store.kind === 'team' && store.active === false
            ? 'var(--c-none)'
            : hue(store.name),
      }}
    >
      {store.name.slice(0, 1)}
    </span>
  );
}
